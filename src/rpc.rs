use crate::metrics::ServerMetrics;
use alloy_primitives::{Bytes, B256, U128, U64};
use alloy_rpc_types_engine::{
    ExecutionPayload, ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadId,
    PayloadStatus,
};
use jsonrpsee::core::{async_trait, ClientError, RegisterMethodError, RpcResult};
use jsonrpsee::http_client::transport::HttpBackend;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::types::error::INVALID_REQUEST_CODE;
use jsonrpsee::types::{ErrorCode, ErrorObject};
use jsonrpsee::RpcModule;
use lru::LruCache;
use op_alloy_rpc_jsonrpsee::traits::{MinerApiExtClient, MinerApiExtServer};
use op_alloy_rpc_types_engine::OpExecutionPayloadEnvelopeV3;
use opentelemetry::global::{self, BoxedSpan, BoxedTracer};
use opentelemetry::trace::{Span, TraceContextExt, Tracer};
use opentelemetry::{Context, KeyValue};
use paste::paste;
use reth_optimism_payload_builder::{OpPayloadAttributes, OpPayloadBuilderAttributes};
use reth_payload_primitives::PayloadBuilderAttributes;
use reth_rpc_layer::{AuthClientLayer, AuthClientService, JwtSecret};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZero;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use clap::{arg, ArgGroup, Parser};
use clap::{
    builder::{PossibleValue, RangedU64ValueParser, TypedValueParser},
    Arg, Args, Command,
};
use std::path::PathBuf;

pub struct ExecutionClient {
    pub client: HttpClient<HttpBackend>,
    pub http_socket: SocketAddr,
    pub auth_client: HttpClient<AuthClientService<HttpBackend>>,
    pub auth_socket: SocketAddr,
}

impl ExecutionClient {
    pub fn new(
        http_addr: IpAddr,
        http_port: u16,
        auth_addr: IpAddr,
        auth_port: u16,
        jwt_secret: JwtSecret,
        timeout: u64,
    ) -> Result<Self, jsonrpsee::core::client::Error> {
        let http_socket = SocketAddr::new(http_addr, http_port);
        let client = HttpClientBuilder::new()
            .request_timeout(Duration::from_millis(timeout))
            .build(format!("http://{}", http_socket))?;

        let auth_layer = AuthClientLayer::new(jwt_secret);
        let auth_socket = SocketAddr::new(auth_addr, auth_port);
        let auth_client = HttpClientBuilder::new()
            .set_http_middleware(tower::ServiceBuilder::new().layer(auth_layer))
            .request_timeout(Duration::from_millis(timeout))
            .build(format!("http://{}", auth_socket))?;

        Ok(Self {
            client,
            http_socket,
            auth_client,
            auth_socket,
        })
    }
}

#[rpc(server, client, namespace = "engine")]
pub trait EngineApi {
    #[method(name = "forkchoiceUpdatedV3")]
    async fn fork_choice_updated_v3(
        &self,
        fork_choice_state: ForkchoiceState,
        payload_attributes: Option<OpPayloadAttributes>,
    ) -> RpcResult<ForkchoiceUpdated>;

    #[method(name = "getPayloadV3")]
    async fn get_payload_v3(
        &self,
        payload_id: PayloadId,
    ) -> RpcResult<OpExecutionPayloadEnvelopeV3>;

    #[method(name = "newPayloadV3")]
    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> RpcResult<PayloadStatus>;
}

#[rpc(server, client, namespace = "eth")]
pub trait EthApi {
    #[method(name = "sendRawTransaction")]
    async fn send_raw_transaction(&self, bytes: Bytes) -> RpcResult<B256>;
}

/*TODO: Remove this in favor of the `MinerApi` from Reth once the
       trait methods are updated to be async
*/
/// Miner namespace rpc interface that can control miner/builder settings
#[rpc(server, client, namespace = "miner")]
pub trait MinerApi {
    /// Sets the extra data string that is included when this miner mines a block.
    ///
    /// Returns an error if the extra data is too long.
    #[method(name = "setExtra")]
    async fn set_extra(&self, record: Bytes) -> RpcResult<bool>;

    /// Sets the minimum accepted gas price for the miner.
    #[method(name = "setGasPrice")]
    async fn set_gas_price(&self, gas_price: U128) -> RpcResult<bool>;

    /// Sets the gaslimit to target towards during mining.
    #[method(name = "setGasLimit")]
    async fn set_gas_limit(&self, gas_price: U128) -> RpcResult<bool>;
}

/// Generates Clap argument structs with a prefix to create a unique namespace when specifing RPC client config via the CLI.
macro_rules! define_rpc_args {
    ($(($name:ident, $prefix:ident)),*) => {
        $(
            paste! {
                #[derive(Parser, Debug, Clone, PartialEq, Eq)]
                #[clap(group(ArgGroup::new(concat!(stringify!($prefix), "_jwt"))
                    .required(true)
                    .multiple(false)
                    .args(&[
                        concat!(stringify!($prefix), "_jwtsecret"),
                        concat!(stringify!($prefix), "_jwtsecret_path")
                    ])
                ))]
                pub struct $name {
                    /// Http server address
                    #[arg(long)]
                    pub [<$prefix _http_addr>]: IpAddr,

                    /// Http server port
                    #[arg(long)]
                    pub [<$prefix _http_port>]: u16,

                    /// Auth server address
                    #[arg(long)]
                    pub [<$prefix _auth_addr>]: IpAddr,

                    /// Auth server port
                    #[arg(long)]
                    pub [<$prefix _auth_port>]: u16,

                    /// Hex encoded JWT secret to authenticate the regular RPC server(s), see `--http.api` and
                    /// `--ws.api`.
                    ///
                    /// This is __not__ used for the authenticated engine-API RPC server, see
                    /// `--authrpc.jwtsecret`.
                    // TODO:
                    #[arg(long, value_name = "HEX", global = true)]
                    pub [<$prefix _jwtsecret>]: Option<JwtSecret>,

                    /// Path to a JWT secret to use for the authenticated engine-API RPC server.
                    ///
                    /// If no path is provided, a secret will be generated and stored in the datadir under
                    /// `<DIR>/<CHAIN_ID>/jwt.hex`. For mainnet this would be `~/.reth/mainnet/jwt.hex` by default.
                    #[arg(long, value_name = "PATH", global = true)]
                    pub [<$prefix _jwtsecret_path>]: Option<PathBuf>,

                    /// Filename for auth IPC socket/pipe within the datadir
                    #[arg(long)]
                    pub [<$prefix _auth_ipc_path>]: Option<String>,

                    /// Timeout for http calls in milliseconds
                    #[arg(long)]
                    pub [<$prefix _timeout>]: u64,
                }
            }
        )*
    };
}

define_rpc_args!((BuilderArgs, builder), (L2ClientArgs, l2));
