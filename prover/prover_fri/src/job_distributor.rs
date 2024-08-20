use jsonrpsee::types::{ErrorCode, ErrorObject};
use std::net::SocketAddr;
use std::sync::Arc;
use anyhow::Result;
use clap::Parser;
use jsonrpsee::server::RpcModule;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tokio::signal;
use tokio::sync::oneshot;
use zksync_prover_fri::cpu_prover_utils::JobDistributor;
use zksync_prover_fri::utils::ProverArtifacts;
use tokio::sync::RwLock;
use zksync_prover_fri_types::ProverJob;
use linked_hash_map::LinkedHashMap;
use zksync_types::basic_fri_types::{AggregationRound, CircuitIdRoundTuple};

const NO_JOB_AVAILABLE_ERROR_CODE: i32 = 1001;
const NO_JOB_AVAILABLE_ERROR_MESSAGE: &str = "No job is currently available.";

const NO_JOB_ID_ERROR_CODE: i32 = 1002;
const NO_JOB_ID_ERROR_MESSAGE: &str = "There is no job with your job id";

const NO_CIRCUIT_ID_ERROR_CODE: i32 = 1003;
const NO_CIRCUIT_ID_ERROR_MESSAGE: &str = "There is no job with your circuit id and aggregation round";

#[derive(Debug, Parser)]
#[command(author = "Matter Labs", version)]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) config_path: Option<std::path::PathBuf>,
    #[arg(long)]
    pub(crate) secrets_path: Option<std::path::PathBuf>,
}

struct Server {
    server_addr: SocketAddr,
    max_size: u32,
    request_id: Arc<AtomicUsize>,
    jobs: Arc<RwLock<LinkedHashMap<(u32, AggregationRound), LinkedHashMap<u32, (ProverJob, Instant)>>>>, // Nested LinkedHashMap
    job_distributor: JobDistributor,
}

impl Server {
    pub async fn new(server_addr: SocketAddr, max_size: u32) -> Result<Self> {
        let opt = Cli::parse();
        let job_distributor = JobDistributor::new(opt.config_path, opt.secrets_path).await.unwrap();
        Ok(Self {
            server_addr,
            max_size,
            request_id: Arc::new(AtomicUsize::new(0)),
            jobs: Arc::new(RwLock::new(LinkedHashMap::new())),
            job_distributor,
        })
    }

    async fn register_methods(self: Arc<Self>, module: &mut RpcModule<()>) -> anyhow::Result<()> {
        let server = self.clone();
        module.register_async_method("get_job", move |_params, _, _| {
            let server = server.clone();
            async move {
                let circuit_ids_for_round_to_be_proven: Vec<CircuitIdRoundTuple> = _params.one()?;
                let _req_id = server.request_id.fetch_add(1, Ordering::SeqCst) as u32;
                let proof_job_option = server.job_distributor.get_next_job(_req_id, circuit_ids_for_round_to_be_proven.clone())
                    .await
                    .map_err(|_e| ErrorObject::from(ErrorCode::InternalError))?;

                // First try to fetch a job from the DB
                if let Some(proof_job) = proof_job_option {
                    let key: (u32, AggregationRound) = (proof_job.circuit_id, proof_job.aggregation_round);
                    let mut jobs = server.jobs.write().await;
                    // Insert the job in the nested linked hash table
                    jobs.entry(key)
                        .or_insert_with(LinkedHashMap::new)
                        .insert(proof_job.job_id, (proof_job.clone(), Instant::now()));

                    println!("Job {} with request id {} inserted.", proof_job.job_id, _req_id);
                    Ok(proof_job)
                } else {
                    println!("No job with the given circuit id(s) and aggregation round(s) was found in the DB. Attempting to retrieve one locally.");
                    let mut jobs = server.jobs.write().await;

                    if circuit_ids_for_round_to_be_proven.is_empty() {
                        // If circuit_ids_for_round_to_be_proven is empty, the client would like to prove any circuit
                        // Iterate through the keys and find the first job
                        if jobs.is_empty() {
                            // If the local nested hashmap is also empty, return an error
                            println!("No job is available.");
                            let error = ErrorObject::owned(
                                NO_JOB_AVAILABLE_ERROR_CODE,
                                NO_JOB_AVAILABLE_ERROR_MESSAGE,
                                None::<()>,
                            );
                            return Err(error);
                        }

                        for (_, job_map) in jobs.iter_mut() {
                            if let Some((job_id, (job, started_job_at))) = job_map.pop_front() {
                                // Reinsert the job to mark it as still pending
                                job_map.insert(job_id, (job.clone(), started_job_at));

                                println!(
                                    "Job {} with request id {}, circuit id {}, and aggregation round {:?} re-inserted from empty circuit_ids_for_round_to_be_proven.",
                                    job.job_id, job.req_id, job.circuit_id, job.aggregation_round
                                );

                                return Ok(job);
                            }
                        }
                    } else {
                        // If circuit_ids_for_round_to_be_proven is not empty, check each tuple in order of appearance in the list
                        for circuit_id_round_tuple in &circuit_ids_for_round_to_be_proven {
                            let key: (u32, AggregationRound) = (
                                circuit_id_round_tuple.circuit_id.into(),
                                circuit_id_round_tuple.aggregation_round.into(),
                            );

                            if let Some(job_map) = jobs.get_mut(&key) {
                                if let Some((job_id, (job, started_job_at))) = job_map.pop_front() {
                                    // Reinsert the job to mark it as still pending
                                    job_map.insert(job_id, (job.clone(), started_job_at));

                                    println!(
                                        "Job {} with request id {}, circuit id {}, and aggregation round {:?} re-inserted.",
                                        job.job_id, job.req_id, job.circuit_id, job.aggregation_round
                                    );

                                    return Ok(job);
                                }
                            }
                        }
                    }

                    // If no job exists locally either, return an error
                    println!("No job with the given circuit id(s) and aggregation round(s) is available locally. Try again later.");
                    let error = ErrorObject::owned(
                        NO_JOB_AVAILABLE_ERROR_CODE,
                        NO_JOB_AVAILABLE_ERROR_MESSAGE,
                        None::<()>,
                    );
                    Err(error)
                }
            }
        })?;

        module.register_async_method("submit_result", move |_params, _, _| {
            let server = self.clone();
            async move {
                let proof_artifact: ProverArtifacts = _params.one()?;
                let key: (u32, AggregationRound) = (proof_artifact.circuit_id, proof_artifact.aggregation_round);
                let mut jobs = server.jobs.write().await;

                if let Some(job_map) = jobs.get_mut(&key) {
                    if let Some((job, started_job_at)) = job_map.remove(&proof_artifact.job_id) {
                        println!(
                            "Received proof artifact for job {} with request id {}.",
                            job.job_id, proof_artifact.request_id
                        );
                        let server_clone = server.clone();

                        // Respond to the client immediately
                        tokio::spawn(async move {
                            let job_id = job.job_id.clone();
                            if JobDistributor::verify_client_proof(proof_artifact.clone(), job).await {
                                let _ = server_clone
                                    .job_distributor
                                    .save_proof_to_db(job_id, proof_artifact, started_job_at)
                                    .await;
                            }
                        });

                        // Clean up the outer map if the inner map is empty
                        if job_map.is_empty() {
                            jobs.remove(&key);
                        }

                        // Respond with success
                        Ok(())
                    } else {
                        println!("There is locally no job with id {}.", proof_artifact.job_id);
                        let error = ErrorObject::owned(
                            NO_JOB_ID_ERROR_CODE,
                            NO_JOB_ID_ERROR_MESSAGE,
                            Some("Job id = ".to_string() + &proof_artifact.job_id.to_string()),
                        );
                        Err(error)
                    }
                } else {
                    println!(
                        "There is locally no job with circuit id {} and aggregation round {:?}.",
                        proof_artifact.circuit_id, proof_artifact.aggregation_round
                    );
                    let error = ErrorObject::owned(
                        NO_CIRCUIT_ID_ERROR_CODE,
                        NO_CIRCUIT_ID_ERROR_MESSAGE,
                        Some(format!(
                            "Circuit id = {}, Aggregation round = {:?}",
                            proof_artifact.circuit_id, proof_artifact.aggregation_round
                        )),
                    );
                    Err(error)
                }
            }
        })?;

        Ok(())
    }

    async fn run(self: Arc<Self>) -> anyhow::Result<()> {
        let server = jsonrpsee::server::Server::builder()
            .max_request_body_size(self.max_size)
            .max_response_body_size(self.max_size)
            .build(self.server_addr)
            .await?;

        let mut module = RpcModule::new(());
        self.clone().register_methods(&mut module).await?;

        let server_handle = server.start(module);

        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
            tx.send(()).unwrap();
        });

        rx.await.unwrap();
        let _ = server_handle.stop();

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let server_addr: SocketAddr = "0.0.0.0:3030".parse()?;
    let max_size = 100 * 1024 * 1024;
    let server = Arc::new(Server::new(server_addr, max_size).await?);
    server.run().await
}