use std::{
    sync::Arc,
    time::{
        Instant, Duration
    },
    io::Write,
    fs::OpenOptions,
};
use anyhow::Context as _;
use zkevm_test_harness::prover_utils::{prove_base_layer_circuit, prove_recursion_layer_circuit};
use zksync_config::configs::FriProverConfig;
use zksync_core_leftovers::temp_config_store::{load_database_secrets, load_general_config};
use zksync_env_config::object_store::ProverObjectStoreConfig;
use zksync_object_store::{ObjectStore, ObjectStoreFactory};
use zksync_prover_dal::{ConnectionPool, Prover as ProverDal};
use zksync_prover_fri_types::{
    circuit_definitions::{
        base_layer_proof_config,
        boojum::{cs::implementations::pow::NoPow, worker::Worker},
        circuit_definitions::{
            base_layer::{ZkSyncBaseLayerCircuit, ZkSyncBaseLayerProof},
            recursion_layer::{ZkSyncRecursionLayerProof, ZkSyncRecursiveLayerCircuit},
        },
        recursion_layer_proof_config,
    },
    CircuitWrapper, FriProofWrapper, ProverJob, ProverServiceDataKey,
    PROVER_PROTOCOL_SEMANTIC_VERSION,
};
use zksync_prover_fri_utils::fetch_next_circuit;
use zksync_types::{
    basic_fri_types::CircuitIdRoundTuple, protocol_version::ProtocolSemanticVersion,
};
use zksync_vk_setup_data_server_fri::{keystore::Keystore, GoldilocksProverSetupData};
use crate::utils::{
    get_setup_data_key, load_setup_data_cache, save_proof, verify_proof, ProverArtifacts,
    SetupLoadMode,
};

pub struct Prover {
    pub config: Arc<FriProverConfig>,
    pub setup_load_mode: SetupLoadMode,
    pub circuit_ids_for_round_to_be_proven: Vec<CircuitIdRoundTuple>,
}

impl Prover {
    #[allow(dead_code)]
    pub fn new(
        prover_config: FriProverConfig,
        circuit_ids_for_round_to_be_proven: Vec<CircuitIdRoundTuple>,
    ) -> anyhow::Result<Self> {
        Ok(Prover {
            config: Arc::new(prover_config.clone()),
            setup_load_mode: load_setup_data_cache(&prover_config)
                .context("load_setup_data_cache()")?,
            circuit_ids_for_round_to_be_proven,
        })
    }

    pub fn prove(&self, job: ProverJob) -> ProverArtifacts {
        let setup_data = get_setup_data(self.setup_load_mode.clone(), job.setup_data_key)
            .context("get_setup_data()")
            .unwrap();
        println!("Proving.");
        let started_at = Instant::now();

        let (proof_wrapper, circuit_id) = match job.circuit_wrapper {
            CircuitWrapper::Base(base_circuit) => (Self::prove_base_layer(
                job.job_id,
                base_circuit.clone(),
                self.config.clone(),
                setup_data,
                job.request_id,
            ), base_circuit.numeric_circuit_type()),
            CircuitWrapper::Recursive(recursive_circuit) => (Self::prove_recursive_layer(
                job.job_id,
                recursive_circuit.clone(),
                self.config.clone(),
                setup_data,
                job.request_id,
            ), recursive_circuit.numeric_circuit_type()),
            CircuitWrapper::BasePartial(_) => panic!("Received partial base circuit"),
        };

        let proving_time = started_at.elapsed();

        println!("Finished proving, took: {:?}", proving_time);
        // Write the proving time to a local file upon successful verification
        if let Err(e) = write_proving_time_to_file(proving_time, circuit_id.into(), job.job_id) {
            eprintln!("Failed to write proving time to file: {}", e);
        }
        ProverArtifacts::new(job.block_number, proof_wrapper, job.job_id, job.request_id)
    }

    fn prove_recursive_layer(
        job_id: u32,
        circuit: ZkSyncRecursiveLayerCircuit,
        _config: Arc<FriProverConfig>,
        artifact: Arc<GoldilocksProverSetupData>,
        request_id: u32,
    ) -> FriProofWrapper {
        let worker = Worker::new();
        let circuit_id = circuit.numeric_circuit_type();
        let proof = prove_recursion_layer_circuit::<NoPow>(
            circuit.clone(),
            &worker,
            recursion_layer_proof_config(),
            &artifact.setup_base,
            &artifact.setup,
            &artifact.setup_tree,
            &artifact.vk,
            &artifact.vars_hint,
            &artifact.wits_hint,
            &artifact.finalization_hint,
        );

        verify_proof(
            &CircuitWrapper::Recursive(circuit),
            &proof,
            &artifact.vk,
            job_id,
            request_id,
        );
        FriProofWrapper::Recursive(ZkSyncRecursionLayerProof::from_inner(circuit_id, proof))
    }

    fn prove_base_layer(
        job_id: u32,
        circuit: ZkSyncBaseLayerCircuit,
        _config: Arc<FriProverConfig>,
        artifact: Arc<GoldilocksProverSetupData>,
        request_id: u32,
    ) -> FriProofWrapper {
        let worker = Worker::new();
        let circuit_id = circuit.numeric_circuit_type();
        let proof = prove_base_layer_circuit::<NoPow>(
            circuit.clone(),
            &worker,
            base_layer_proof_config(),
            &artifact.setup_base,
            &artifact.setup,
            &artifact.setup_tree,
            &artifact.vk,
            &artifact.vars_hint,
            &artifact.wits_hint,
            &artifact.finalization_hint,
        );

        verify_proof(
            &CircuitWrapper::Base(circuit),
            &proof,
            &artifact.vk,
            job_id,
            request_id,
        );
        FriProofWrapper::Base(ZkSyncBaseLayerProof::from_inner(circuit_id, proof))
    }
}

pub struct JobDistributor {
    pub prover_config: FriProverConfig,
    pub object_store: Arc<dyn ObjectStore>,
    pub prover_connection_pool: ConnectionPool<ProverDal>,
    pub protocol_version: ProtocolSemanticVersion,
    pub blob_store: Arc<dyn ObjectStore>,
    pub public_blob_store: Option<Arc<dyn ObjectStore>>,
}

impl JobDistributor {
    pub async fn new(
        config_path: Option<std::path::PathBuf>,
        secrets_path: Option<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        let general_config = load_general_config(config_path).context("general config")?;
        let prover_config = general_config.prover_config.context("fri_prover config")?;
        let database_secrets = load_database_secrets(secrets_path).context("database secrets")?;

        let prover_connection_pool =
            ConnectionPool::<ProverDal>::singleton(database_secrets.prover_url()?)
                .build()
                .await
                .context("failed to build a prover_connection_pool")?;
        let object_store_config = ProverObjectStoreConfig(
            prover_config
                .clone()
                .prover_object_store
                .context("object store")?,
        );
        let object_store = ObjectStoreFactory::new(object_store_config.0.clone())
            .create_store()
            .await?;
        let store_factory = ObjectStoreFactory::new(object_store_config.0);
        let public_object_store_config = prover_config
            .public_object_store
            .clone()
            .context("public object store config")?;

        let public_blob_store = match prover_config.shall_save_to_public_bucket {
            false => None,
            true => Some(
                ObjectStoreFactory::new(public_object_store_config)
                    .create_store()
                    .await?,
            ),
        };

        Ok(JobDistributor {
            prover_config,
            object_store,
            prover_connection_pool,
            protocol_version: PROVER_PROTOCOL_SEMANTIC_VERSION,
            blob_store: store_factory.create_store().await?,
            public_blob_store,
        })
    }

    pub async fn verify_client_proof(proof_artifact: ProverArtifacts, job: ProverJob) -> bool {
        // Modify setup data key of job if it is a Node Aggregation Job, otherwise return same key
        let setup_data_key = get_setup_data_key(job.setup_data_key);

        let is_valid = match (proof_artifact.proof_wrapper.clone(), job.circuit_wrapper) {
            (FriProofWrapper::Base(proof), CircuitWrapper::Base(base_circuit)) => {
                assert!(base_circuit.numeric_circuit_type() == setup_data_key.circuit_id);
                // Try to load the base layer verification key
                let v_k = match Keystore::default()
                    .load_base_layer_verification_key(get_setup_data_key(setup_data_key).circuit_id)
                {
                    Ok(vk) => vk.into_inner(),
                    Err(_) => return false,
                };
                verify_proof(
                    &CircuitWrapper::Base(base_circuit),
                    &proof.into_inner(),
                    &v_k,
                    job.job_id,
                    proof_artifact.request_id,
                )
            }
            (FriProofWrapper::Recursive(proof), CircuitWrapper::Recursive(recursive_circuit)) => {
                assert!(recursive_circuit.numeric_circuit_type() == setup_data_key.circuit_id);
                // Try to load the recursive layer verification key
                let v_k = match Keystore::default()
                    .load_recursive_layer_verification_key(setup_data_key.circuit_id)
                {
                    Ok(vk) => vk.into_inner(),
                    Err(_) => return false,
                };
                verify_proof(
                    &CircuitWrapper::Recursive(recursive_circuit),
                    &proof.into_inner(),
                    &v_k,
                    job.job_id,
                    proof_artifact.request_id,
                )
            }
            _ => false, // Handle the mismatched case by returning false
        };
        is_valid
    }

    pub async fn get_next_job(
        &self,
        _req_id: u32,
        circuit_ids_for_round_to_be_proven: Vec<CircuitIdRoundTuple>,
    ) -> anyhow::Result<Option<ProverJob>> {
        let mut storage = self.prover_connection_pool.connection().await.unwrap();
        let Some(job) = fetch_next_circuit(
            &mut storage,
            &*self.object_store,
            &circuit_ids_for_round_to_be_proven,
            &self.protocol_version,
            _req_id,
        )
        .await
        else {
            return Ok(None);
        };
        Ok(Some(job))
    }

    pub async fn save_proof_to_db(
        &self,
        job_id: u32,
        artifacts: ProverArtifacts,
        started_at: Instant,
    ) -> anyhow::Result<()> {
        // Error handling when getting connection
        let mut storage_processor = match self.prover_connection_pool.connection().await {
            Ok(conn) => conn,
            Err(e) => {
                println!("Failed to get connection for job {}: {:?}", job_id, e);
                return Err(anyhow::anyhow!(
                    "Failed to get connection for job {}: {:?}",
                    job_id,
                    e
                ));
            }
        };

        save_proof(
            job_id,
            started_at,
            artifacts,
            &*self.blob_store,
            self.public_blob_store.as_deref(),
            self.prover_config.shall_save_to_public_bucket,
            &mut storage_processor,
            self.protocol_version,
        )
        .await;
        println!(
            "Wrote to DB that job {} has been successfully completed.",
            job_id
        );
        Ok(())
    }
}

fn write_proving_time_to_file(proving_time: Duration, circuit_id: u32, job_id: u32) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("proving_times.txt")?;
    // Format the data as a comma-separated line
    let log_entry = format!(
        "{:?},{},{}\n",
        proving_time,
        circuit_id,
        job_id,
    );
    file.write_all(log_entry.as_bytes())?;
    Ok(())
}

pub fn write_username_to_file(username: &str, job_id: u32, started_job_at: Instant) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("verified_provers.txt")?;

    // Format the data as a comma-separated line
    let log_entry = format!(
        "{},{},{:?},{:?}\n",
        username,
        job_id,
        started_job_at,
        Instant::now(),
    );

    file.write_all(log_entry.as_bytes())?;
    Ok(())
}

pub fn get_setup_data(
    setup_load_mode: SetupLoadMode,
    key: ProverServiceDataKey,
) -> anyhow::Result<Arc<GoldilocksProverSetupData>> {
    let key = get_setup_data_key(key);
    Ok(match setup_load_mode {
        SetupLoadMode::FromMemory(cache) => cache
            .get(&key)
            .context("Setup data not found in cache")?
            .clone(),
        SetupLoadMode::FromDisk => {
            let started_at = Instant::now();
            let keystore = Keystore::default();
            let artifact: GoldilocksProverSetupData = keystore
                .load_cpu_setup_data_for_circuit_type(key.clone())
                .context("get_cpu_setup_data_for_circuit_type()")?;
            println!("Setup data load time, took: {:?}", started_at.elapsed());
            Arc::new(artifact)
        }
    })
}

pub fn parse_circuit_ids_rounds(s: &str) -> Result<Vec<CircuitIdRoundTuple>, anyhow::Error> {
    s.split("),(")
        .map(|pair| {
            let cleaned = pair.trim_matches(|c| c == '(' || c == ')');
            let nums: Vec<&str> = cleaned.split(',').collect();
            if nums.len() != 2 {
                return Err(anyhow::anyhow!("Invalid tuple format: {}", pair));
            }
            let first: u8 = nums[0]
                .trim()
                .parse::<u32>()
                .context("Failed to parse first element of tuple")?
                .try_into()
                .context("Failed to convert first element to u8")?;
            let second: u8 = nums[1]
                .trim()
                .parse::<u32>()
                .context("Failed to parse second element of tuple")?
                .try_into()
                .context("Failed to convert second element to u8")?;
            Ok(CircuitIdRoundTuple::new(first, second))
        })
        .collect()
}