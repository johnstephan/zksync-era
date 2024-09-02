# Community Proving with Zksync-Era

## Introduction

This project allows users to perform community proving using Zksync-Era. Follow the steps below to set up the necessary
environment and run the prover.

## Prerequisites

- Ubuntu 20.04+ or MacOS
- 32 GB RAM (or at least 16 GB with a lot of swap space).
- Bash shell
- wget or curl

> **Note**: MacOS usually provides sufficient swap space. For Ubuntu, see
> [this guide](https://askubuntu.com/questions/178712/how-to-increase-swap-space) if you need to increase swap space.

## 1. Get The Code

Clone this repository, checking out the `community-proving` branch.

```bash
git clone -b community-proving https://github.com/johnstephan/zksync-era.git
```

## 2. Install Required Components

For Ubuntu users:

```bash
chmod +x install-ubuntu.sh
./install-ubuntu.sh
```

For Mac users:

```bash
chmod +x install-mac.sh
./install-mac.sh
```

When the script is done (for either of Ubuntu or Mac), restart the terminal to reload the shell configuration.

> **Note**: These scripts require some manual interactions. If any issues arise, refer to the step-by-step guides
> [for Mac](./setup_instructions_mac.md) and [for Ubuntu](./setup_instructions_ubuntu.md).

## 3. Download Proving and Verification Keys

Before running the prover, download the necessary proving and verification keys:

```bash
wget http://34.29.79.81:8000/setup_basic_1_data.bin
wget http://34.29.79.81:8000/verification_basic_1_key.json
```

Or using curl:

```bash
curl -O http://34.29.79.81:8000/setup_basic_1_data.bin
curl -O http://34.29.79.81:8000/verification_basic_1_key.json
```

Place these files in the following directory: `zksync-era/prover/vk_setup_data_generator_server_fri/data`.

> **Note**: The file `setup_basic_1_data.bin` is over 13GB in size and will take long to download, but this only needs
> to happen once. Performing proving jobs later only requires little network bandwidth. If you have constraints on
> network bandwidth or the amount of downloaded data, see the [advanced instructions](./README_advanced.md) for
> generating the keys locally.

## 4. Run the Prover

Once everything is set up, run the prover with the following command:

```bash
cd prover
chmod +x run_prover.sh
./run_prover.sh --server-url http://34.29.79.81:3030
```

> **Note**: On Mac, the prover may crash unexpectedly. If it does, the script will automatically relaunch the prover.

**_Important_**: This guide is designed for users new to community proving with Zksync-Era, and enables users to execute
the most common prover job (Circuit ID 1, Round 0) in Zksync-Era. If you would like to be able to execute more prover
jobs, please refer to the [Advanced README](./README_advanced.md). If you would like to be able to execute **all**
prover jobs, do the following steps.

First, generate keys for all possible circuits and rounds using the following command:

```bash
cd prover
./setup.sh
```

> **Note**: Generating all keys will require approximately 400 GB of disk space, and approximately an hour to complete.

Then, run the prover with the following command:

```bash
chmod +x run_prover.sh
./run_prover.sh --server-url http://34.29.79.81:3030 --circuit-ids "all"
```

where setting circuit-ids to "all" implies support for all prover jobs (i.e., you are willing to execute any prover
job).

## Additional Resources

- [Setup Instructions for Mac](./setup_instructions_mac.md)
- [Setup Instructions for Ubuntu](./setup_instructions_ubuntu.md)

## Contact

If you have any questions, feel free to reach out via email at [jst@matterlabs.dev](mailto:jst@matterlabs.dev), or open
an issue on GitHub.
