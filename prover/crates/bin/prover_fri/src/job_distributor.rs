use std::{collections::HashMap, net::SocketAddr, sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
}, time::Instant};
use anyhow::Result;
use clap::Parser;
use jsonrpsee::{
    server::RpcModule,
    types::{ErrorCode, ErrorObject},
};
use tokio::{
    signal,
    sync::{oneshot, RwLock},
};
use zksync_prover_fri::{
    cpu_prover_utils::{
        JobDistributor, write_username_to_file
    },
    utils::ProverArtifacts};
use zksync_prover_fri_types::ProverJob;
use zksync_types::basic_fri_types::CircuitIdRoundTuple;
use serde::Deserialize;

#[derive(Deserialize)]
struct SubmitResultParams {
    username: String,
    proof_artifact: ProverArtifacts,
}

const NO_JOB_AVAILABLE_ERROR_CODE: i32 = 1001;
const NO_JOB_AVAILABLE_ERROR_MESSAGE: &str = "No job is currently available.";

const NO_JOB_ID_ERROR_CODE: i32 = 1002;
const NO_JOB_ID_ERROR_MESSAGE: &str = "There is no job with your job id";

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
    jobs: Arc<RwLock<HashMap<u32, HashMap<u32, (ProverJob, Instant)>>>>,
    job_distributor: JobDistributor,
}

impl Server {
    pub async fn new(server_addr: SocketAddr, max_size: u32) -> Result<Self> {
        let opt = Cli::parse();
        let job_distributor = JobDistributor::new(opt.config_path, opt.secrets_path)
            .await
            .unwrap();
        Ok(Self {
            server_addr,
            max_size,
            request_id: Arc::new(AtomicUsize::new(0)),
            jobs: Arc::new(RwLock::new(HashMap::new())),
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
                let proof_job_option = server
                    .job_distributor
                    .get_next_job(_req_id, circuit_ids_for_round_to_be_proven)
                    .await
                    .map_err(|_e| ErrorObject::from(ErrorCode::InternalError))?;

                if let Some(proof_job) = proof_job_option {
                    // Insert the job into the nested hashmap
                    let mut jobs = server.jobs.write().await;
                    let job_entry = jobs.entry(proof_job.job_id).or_insert_with(HashMap::new);
                    job_entry.insert(_req_id, (proof_job.clone(), Instant::now()));
                    println!(
                        "Job {} with request id {} inserted.",
                        proof_job.job_id, _req_id
                    );
                    Ok(proof_job)
                } else {
                    println!("No job with the provided (circuit_id, aggregation_round) is currently available. Please try again later.");
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
                // Deserialize the response JSON object into the `SubmitResultParams` struct
                let response: SubmitResultParams = _params.one()?;

                // Access the fields
                let username = response.username;
                let proof_artifact = response.proof_artifact;
                let job_id = proof_artifact.job_id;
                let request_id = proof_artifact.request_id;

                let mut jobs = server.jobs.write().await;
                if let Some(job_map) = jobs.get_mut(&job_id) {
                    if let Some((job, started_job_at)) = job_map.remove(&request_id) {
                        println!(
                            "Received from {} the proof artifact for job {} with request id {}.",
                            username, job_id, request_id
                        );

                        // Make all other client requests obsolete
                        jobs.remove(&job_id);

                        let server_clone = server.clone();
                        // Respond to the client immediately
                        tokio::spawn(async move {
                            if JobDistributor::verify_client_proof(proof_artifact.clone(), job).await {
                                let _ = server_clone
                                    .job_distributor
                                    .save_proof_to_db(job_id, proof_artifact, started_job_at)
                                    .await;

                                // Write the username to a local file upon successful verification
                                if let Err(e) = write_username_to_file(&username, job_id, started_job_at) {
                                    eprintln!("Failed to write username to file: {}", e);
                                }
                            }
                        });

                        // Respond with success
                        Ok(())
                    } else {
                        println!(
                            "There is currently no job with request id {} for job id {}.",
                            request_id, job_id
                        );
                        let error = ErrorObject::owned(
                            NO_JOB_ID_ERROR_CODE,
                            NO_JOB_ID_ERROR_MESSAGE,
                            Some("Job id = ".to_string() + &job_id.to_string()),
                        );
                        Err(error)
                    }
                } else {
                    println!(
                        "This job is obsolete (already been proven), or no job with job id {} exists. Please try again later.",
                        job_id
                    );
                    let error = ErrorObject::owned(
                        NO_JOB_ID_ERROR_CODE,
                        NO_JOB_ID_ERROR_MESSAGE,
                        Some("Job id = ".to_string() + &job_id.to_string()),
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