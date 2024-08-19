#!/bin/bash

# Exit immediately if any of the below commands fails
set -e

# Remember local directory as the root of community proving
ZKSYNC_HOME=$(pwd)

# Install Homebrew
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
(echo; echo 'eval "$(/opt/homebrew/bin/brew shellenv)"') >> $HOME/.zprofile
eval "$(/opt/homebrew/bin/brew shellenv)"

# Install packages
brew install cmake

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
echo '. "$HOME/.cargo/env"' >> $HOME/.zprofile
. "$HOME/.cargo/env"

# Install NVM
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.1/install.sh | bash
echo "export NVM_DIR=\"$HOME/.nvm\"" >> $HOME/.zprofile
echo '[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"  # This loads nvm' >> $HOME/.zprofile
export NVM_DIR="$HOME/.nvm"

# Execute the nvm script in the home directory (assuming there is no .nvmrc there).
# See https://github.com/nvm-sh/nvm/issues/1985
working_dir=$(pwd)
cd
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"
cd $working_dir

# Install Node and Yarn
nvm install 18
npm install -g yarn
yarn set version 1.22.19

# Set the zksync environment variables
echo "export ZKSYNC_HOME=\"$ZKSYNC_HOME\"" >> $HOME/.zprofile
echo 'export PATH="$ZKSYNC_HOME/bin:$PATH"' >> $HOME/.zprofile
export ZKSYNC_HOME="$ZKSYNC_HOME"
export PATH="$ZKSYNC_HOME/bin:$PATH"

# Init ZKsync Era
zk
