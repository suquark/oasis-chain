// Copyright 2015-2018 Parity Technologies (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

//! Web3 rpc implementation.
use hash::keccak;
use jsonrpc_core::Result;
use parity_rpc::v1::{
    traits::Web3,
    types::{Bytes, H256},
};

/// Web3 rpc implementation.
pub struct Web3Client;

impl Web3Client {
    /// Creates new Web3Client.
    pub fn new() -> Self {
        Web3Client
    }
}

impl Web3 for Web3Client {
    fn client_version(&self) -> Result<String> {
        Ok(format!(
            "oasis/{}/{}/{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_GIT_REV"), // set in build.rs
        ))
    }

    fn sha3(&self, data: Bytes) -> Result<H256> {
        Ok(keccak(&data.0).into())
    }
}
