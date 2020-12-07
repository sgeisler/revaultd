use crate::{revaultd::VaultStatus, threadmessages::*};
use common::VERSION;

use revault_tx::bitcoin::{
    hashes::hex::{Error as FromHexError, FromHex},
    Txid,
};

use std::{
    process,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
};

use jsonrpc_core::Error as JsonRpcError;
use jsonrpc_derive::rpc;
use serde_json::json;

#[derive(Clone)]
pub struct JsonRpcMetaData {
    pub tx: Sender<ThreadMessageIn>,
    pub shutdown: Arc<AtomicBool>,
}
impl jsonrpc_core::Metadata for JsonRpcMetaData {}

impl JsonRpcMetaData {
    pub fn from_tx(tx: Sender<ThreadMessageIn>) -> Self {
        JsonRpcMetaData {
            tx,
            shutdown: Arc::from(AtomicBool::from(false)),
        }
    }

    pub fn is_shutdown(&self) -> bool {
        return self.shutdown.load(Ordering::Relaxed);
    }

    pub fn shutdown(&self) {
        // Relaxed is fine, worse case we just stop at the next iteration on ARM
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

#[rpc(server)]
pub trait RpcApi {
    type Metadata;

    /// Stops the daemon
    #[rpc(meta, name = "stop")]
    fn stop(&self, meta: Self::Metadata) -> jsonrpc_core::Result<()>;

    /// Get informations about the daemon
    #[rpc(meta, name = "getinfo")]
    fn getinfo(&self, meta: Self::Metadata) -> jsonrpc_core::Result<serde_json::Value>;

    /// Get a list of current vaults, which can be sorted by txids or status
    #[rpc(meta, name = "listvaults")]
    fn listvaults(
        &self,
        meta: Self::Metadata,
        status: Option<String>,
        txids: Option<Vec<String>>,
    ) -> jsonrpc_core::Result<serde_json::Value>;
}

pub struct RpcImpl;
impl RpcApi for RpcImpl {
    type Metadata = JsonRpcMetaData;

    fn stop(&self, meta: JsonRpcMetaData) -> jsonrpc_core::Result<()> {
        meta.shutdown();
        meta.tx
            .send(ThreadMessageIn::Rpc(RpcMessageIn::Shutdown))
            .unwrap();
        Ok(())
    }

    fn getinfo(&self, meta: Self::Metadata) -> jsonrpc_core::Result<serde_json::Value> {
        let (response_tx, response_rx) = mpsc::sync_channel(0);
        meta.tx
            .send(ThreadMessageIn::Rpc(RpcMessageIn::GetInfo(response_tx)))
            .unwrap_or_else(|e| {
                log::error!("Sending 'getinfo' to main thread: {:?}", e);
                process::exit(1);
            });
        let (net, height, progress) = response_rx.recv().unwrap_or_else(|e| {
            log::error!("Receiving 'getinfo' result from main thread: {:?}", e);
            process::exit(1);
        });

        Ok(json!({
            "version": VERSION.to_string(),
            "network": net,
            "blockheight": height,
            "sync": progress,
        }))
    }

    fn listvaults(
        &self,
        meta: Self::Metadata,
        status: Option<String>,
        txids: Option<Vec<String>>,
    ) -> jsonrpc_core::Result<serde_json::Value> {
        let status = if let Some(status) = status {
            Some(VaultStatus::from_str(&status).map_err(|_| {
                JsonRpcError::invalid_params(format!("'{}' is not a valid vault status", &status))
            })?)
        } else {
            None
        };
        let txids = if let Some(txids) = txids {
            Some(
                txids
                    .into_iter()
                    .map(|tx_str| {
                        Txid::from_hex(&tx_str).map_err(|e| {
                            JsonRpcError::invalid_params(format!(
                                "'{}' is not a valid txid ({})",
                                &tx_str,
                                e.to_string()
                            ))
                        })
                    })
                    .collect::<jsonrpc_core::Result<Vec<Txid>>>()?,
            )
        } else {
            None
        };

        let (response_tx, response_rx) = mpsc::sync_channel(0);
        meta.tx
            .send(ThreadMessageIn::Rpc(RpcMessageIn::ListVaults(
                (status, txids),
                response_tx,
            )))
            .unwrap_or_else(|e| {
                log::error!("Sending 'listvaults' to main thread: {:?}", e);
                process::exit(1);
            });
        let vaults = response_rx.recv().unwrap_or_else(|e| {
            log::error!("Receiving 'listvaults' result from main thread: {:?}", e);
            process::exit(1);
        });

        Ok(json!({ "vaults": vaults }))
    }
}