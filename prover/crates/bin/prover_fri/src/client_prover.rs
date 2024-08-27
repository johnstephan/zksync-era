use std::time::Instant;
use anyhow::Context as _;
use clap::Parser;
use jsonrpsee::{core::client::ClientT, http_client::HttpClientBuilder, rpc_params};
use tokio;
use zksync_core_leftovers::temp_config_store::load_general_config;
use zksync_prover_fri::cpu_prover_utils::{parse_circuit_ids_rounds, Prover};
use zksync_prover_fri_types::ProverJob;
use zksync_prover_fri_utils::get_all_circuit_id_round_tuples_for;

#[derive(Debug, Parser)]
#[command(author = "Matter Labs", version)]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) config_path: Option<std::path::PathBuf>,
    #[arg(long)]
    pub(crate) secrets_path: Option<std::path::PathBuf>,
    #[arg(long)]
    pub(crate) server_url: String,
    #[arg(long, default_value = "anonymous")]
    pub(crate) username: String,
    #[arg(long, default_value = "(1,0)")]
    pub(crate) circuit_ids_rounds: String,
}

struct Client {
    client: jsonrpsee::http_client::HttpClient,
    client_prover: Prover,
    username: String,
}

impl Client {
    pub async fn new(
        server_url: &str,
        max_size: u32,
        circuit_ids_rounds: String,
        username: String,
    ) -> anyhow::Result<Self> {
        let client = HttpClientBuilder::default()
            .max_request_size(max_size)
            .max_response_size(max_size)
            .build(server_url)?;

        let general_config =
            load_general_config(Cli::parse().config_path.clone()).context("general config")?;
        let prover_config = general_config.prover_config.context("fri_prover config")?;

        // Determine how to set circuit_ids_for_round_to_be_proven based on the input
        let circuit_ids_for_round_to_be_proven = if circuit_ids_rounds == "all" {
            let circuit_ids = general_config
                .prover_group_config
                .expect("prover_group_config")
                .get_circuit_ids_for_group_id(prover_config.specialized_group_id)
                .unwrap_or_default();
            get_all_circuit_id_round_tuples_for(circuit_ids)
        } else {
            parse_circuit_ids_rounds(&circuit_ids_rounds)?
        };

        let client_prover = Prover::new(prover_config, circuit_ids_for_round_to_be_proven).unwrap();
        Ok(Self {
            client,
            client_prover,
            username,
        })
    }

    pub async fn poll_for_job(&self) -> anyhow::Result<()> {

        // Record timestamp
        let _job_request_timestamp = Instant::now();

        // Request a job
        let circuit_ids_json = serde_json::to_value(
            self.client_prover
                .circuit_ids_for_round_to_be_proven
                .clone(),
        )?;
        let response: Result<ProverJob, _> = self
            .client
            .request("get_job", rpc_params![circuit_ids_json])
            .await;

        match response {

            Ok(job) => {
                println!(
                    "Have to execute job {} with request id {}.",
                    job.job_id, job.request_id
                );
                let proof_artifact = self.client_prover.prove(job);
                let _proof_submission_timestamp = Instant::now();

                // Include the different fields in the Json object: username, proof artifact
                let result_json = serde_json::json!({
                    "username": self.username,
                    "proof_artifact": proof_artifact,
                });

                let submit_response: Result<(), _> = self
                    .client
                    .request("submit_result", rpc_params![result_json])
                    .await;

                match submit_response {
                    Ok(_) => println!("Proof submitted and verified successfully."),
                    Err(e) => eprintln!("Failed to submit proof: {}.", e),
                }
            }
            Err(e) => eprintln!("{}", e),
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opt = Cli::parse();
    let server_url = &opt.server_url;
    let max_size: u32 = 100 * 1024 * 1024;
    let circuit_ids_rounds = opt.circuit_ids_rounds;
    let username = opt.username;
    let client = Client::new(server_url, max_size, circuit_ids_rounds, username).await?;
    client.poll_for_job().await?;
    Ok(())
}