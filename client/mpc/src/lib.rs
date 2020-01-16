use std::{collections::BTreeMap, fmt::Debug, pin::Pin, sync::Arc};

use curv::{
	cryptographic_primitives::{proofs::sigma_dlog::DLogProof, secret_sharing::feldman_vss::VerifiableSS},
	elliptic::curves::traits::ECPoint,
	FE, GE,
};
use futures::{
	channel::mpsc,
	future::{ready, select, FutureExt, TryFutureExt},
	prelude::{Future, Sink, Stream},
	stream::StreamExt,
	task::{Context, Poll, Spawn},
};
use log::{error, info};
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2018::party_i::{
	KeyGenBroadcastMessage1 as KeyGenCommit, KeyGenDecommitMessage1 as KeyGenDecommit, Keys, Parameters, PartyPrivate,
	SharedKeys, SignKeys,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use sc_client_api::{blockchain::HeaderBackend, Backend, BlockchainEvents};
use sc_network::{NetworkService, NetworkStateInfo, PeerId};
use sc_network_gossip::Network as GossipNetwork;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::{Error as ClientError, Result as ClientResult};
use sp_core::{offchain::OffchainStorage, Blake2Hasher, H256};
use sp_offchain::STORAGE_PREFIX;
use sp_runtime::generic::{BlockId, OpaqueDigestItemId};
use sp_runtime::traits::{Block as BlockT, Header};

use sp_mpc::{get_storage_key, ConsensusLog, MpcApi, MpcRequest, OffchainStorageType, MPC_ENGINE_ID};

mod communication;
mod periodic_stream;
mod signer;

use communication::{
	gossip::{GossipEra, GossipMessage, MessageWithSender},
	message::{ConfirmPeersMessage, KeyGenMessage, PeerIndex, SigGenMessage},
	NetworkBridge,
};
use periodic_stream::PeriodicStream;
use signer::Signer;

pub trait Network<B: BlockT>: GossipNetwork<B> + Clone + Send + 'static {
	fn local_peer_id(&self) -> PeerId;
}

impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>>
where
	B: BlockT,
	S: sc_network::specialization::NetworkSpecialization<B>,
	H: sc_network::ExHashT,
{
	fn local_peer_id(&self) -> PeerId {
		NetworkService::local_peer_id(self)
	}
}

#[derive(Debug)]
pub enum Error {
	Network(String),
	Periodic,
	Client(ClientError),
	Rebuild,
}

#[derive(Clone)]
pub struct NodeConfig {
	pub duration: u64,
	pub threshold: u16,
	pub players: u16,
}

impl NodeConfig {
	pub fn get_params(&self) -> Parameters {
		Parameters {
			threshold: self.threshold,
			share_count: self.players,
		}
	}
}

#[derive(Clone, Serialize, Deserialize)]
pub struct KeyGenState {
	pub req_id: u64,
	pub complete: bool,
	pub local_key: Option<Keys>,
	pub commits: BTreeMap<PeerIndex, KeyGenCommit>,
	pub decommits: BTreeMap<PeerIndex, KeyGenDecommit>,
	pub vsss: BTreeMap<PeerIndex, VerifiableSS>,
	pub secret_shares: BTreeMap<PeerIndex, FE>,
	pub proofs: BTreeMap<PeerIndex, DLogProof>,
	pub shared_keys: Option<SharedKeys>,
}

impl KeyGenState {
	pub fn shared_public_key(&self) -> Option<Vec<u8>> {
		self.shared_keys.clone().map(|sk| sk.y.pk_to_key_slice())
	}

	pub fn reset(&mut self) {
		*self = Self::default();
	}
}

impl Default for KeyGenState {
	fn default() -> Self {
		Self {
			req_id: 0,
			complete: false,
			local_key: None,
			commits: BTreeMap::new(),
			decommits: BTreeMap::new(),
			vsss: BTreeMap::new(),
			secret_shares: BTreeMap::new(),
			proofs: BTreeMap::new(),
			shared_keys: None,
		}
	}
}

#[derive(Debug)]
pub struct SigGenState {}

pub(crate) struct Environment<Client, Block: BlockT, Storage> {
	pub client: Arc<Client>,
	pub config: NodeConfig,
	pub bridge: NetworkBridge<Block>,
	pub state: Arc<RwLock<KeyGenState>>, // HashMap id -> key gen state
	pub offchain: Arc<RwLock<Storage>>,
}

struct KeyGenWork<Client, Block: BlockT, Storage> {
	worker: Pin<Box<dyn Future<Output = Result<(), Error>> + Send + Unpin>>,
	env: Arc<Environment<Client, Block, Storage>>,
	mpc_arg_rx: mpsc::UnboundedReceiver<MpcRequest>,
}

impl<Client, Block, Storage> KeyGenWork<Client, Block, Storage>
where
	Client: HeaderBackend<Block> + ProvideRuntimeApi<Block> + Send + Sync + 'static,
	<Client as ProvideRuntimeApi<Block>>::Api: MpcApi<Block>,
	Block: BlockT<Hash = H256> + Unpin,
	Block::Hash: Ord,
	Storage: OffchainStorage + 'static,
{
	fn new(
		client: Arc<Client>,
		config: NodeConfig,
		bridge: NetworkBridge<Block>,
		offchain: Storage,
		mpc_arg_rx: mpsc::UnboundedReceiver<MpcRequest>,
	) -> Self {
		let state = KeyGenState::default();

		let env = Arc::new(Environment {
			client,
			config,
			bridge,
			state: Arc::new(RwLock::new(state)),
			offchain: Arc::new(RwLock::new(offchain)),
		});

		let mut work = Self {
			worker: Box::pin(futures::future::pending()),
			env,
			mpc_arg_rx,
		};
		work.rebuild();
		work
	}

	fn rebuild(&mut self) {
		let (incoming, outgoing) = global_comm(&self.env.bridge, self.env.config.duration);
		let signer = Signer::new(self.env.clone(), incoming, outgoing);
		self.worker = Box::pin(signer);
	}

	fn handle_command(&mut self, command: MpcRequest) {
		match command {
			MpcRequest::KeyGen(id) => {
				let mut state = self.env.state.write();
				state.req_id = id;
				self.env.bridge.start_key_gen(id);
			}
			MpcRequest::SigGen(_req_id, pk_id, _) => {
				let id = BlockId::hash(self.env.client.info().best_hash);
				let pk = self.env.client.runtime_api().get_public_key(&id, pk_id);
				info!("get pk from onchain {:x?} of {:?}", pk, pk_id);
			}
		}
	}
}

impl<Client, Block, Storage> Future for KeyGenWork<Client, Block, Storage>
where
	Client: HeaderBackend<Block> + ProvideRuntimeApi<Block> + Send + Sync + 'static,
	<Client as ProvideRuntimeApi<Block>>::Api: MpcApi<Block>,
	Block: BlockT<Hash = H256> + Unpin,
	Block::Hash: Ord,
	Storage: OffchainStorage + 'static,
{
	type Output = Result<(), Error>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
		match self.mpc_arg_rx.poll_next_unpin(cx) {
			Poll::Pending => {}
			Poll::Ready(None) => {
				// impossible
				return Poll::Ready(Ok(()));
			}
			Poll::Ready(Some(command)) => {
				self.handle_command(command);
				futures01::task::current().notify();
			}
		}

		match self.worker.poll_unpin(cx) {
			Poll::Pending => {
				{
					info!("POLLING IN KEYGEN WORK: Pending");
					let state = self.env.state.read();
					let validator = self.env.bridge.validator.inner.read();

					if state.complete {
						let mut offchain_storage = self.env.offchain.write();

						let ls = state.clone();
						let raw_state = bincode::serialize(&ls).unwrap();
						let key_for_state = get_storage_key(state.req_id, OffchainStorageType::LocalKeyState);
						offchain_storage.set(STORAGE_PREFIX, &key_for_state, &raw_state);

						let pk = state.shared_keys.clone().unwrap();
						let raw_pk = state.shared_public_key().unwrap();
						let key_for_pk = get_storage_key(state.req_id, OffchainStorageType::SharedPublicKey);

						println!("  pk: {:?}, raw_pk: {:x?}", b"", raw_pk);
						offchain_storage.set(STORAGE_PREFIX, &key_for_pk, &raw_pk);
					}

					println!(
						"INDEX {:?} state: commits {:?} decommits {:?} vss {:?} ss {:?} proof {:?} has key {:?} has shared_key {:?} complete {:?} peers hash {:?}",
						validator.get_local_index(),
						state.commits.len(),
						state.decommits.len(),
						state.vsss.len(),
						state.secret_shares.len(),
						state.proofs.len(),
						state.local_key.is_some(),
						state.shared_keys.is_some(),
						state.complete,
						validator.get_peers_hash()
					);
				};

				return Poll::Pending;
			}
			Poll::Ready(Ok(())) => {
				info!("POLLING IN KEYGEN WORK: Ok");
				return Poll::Ready(Ok(()));
			}
			Poll::Ready(Err(e)) => match e {
				Error::Rebuild => {
					info!("POLLING IN KEYGEN WORK: Error::Rebuild");
					self.rebuild();
					futures01::task::current().notify();
					return Poll::Pending;
				}
				e @ _ => {
					info!("POLLING IN KEYGEN WORK: Error::{:?}", e);
					return Poll::Ready(Err(e));
				}
			},
		}
	}
}

fn global_comm<Block>(
	bridge: &NetworkBridge<Block>,
	duration: u64,
) -> (
	impl Stream<Item = MessageWithSender>,
	impl Sink<MessageWithSender, Error = Error>,
)
where
	Block: BlockT<Hash = H256>,
{
	let (global_in, global_out) = bridge.global();
	let global_in = PeriodicStream::<_, MessageWithSender>::new(global_in, duration);

	(global_in, global_out)
}

pub fn run_mpc_task<Client, Block, B, N, Ex>(
	client: Arc<Client>,
	backend: Arc<B>,
	network: N,
	executor: Ex,
) -> ClientResult<impl Future<Output = ()>>
where
	Client: HeaderBackend<Block> + ProvideRuntimeApi<Block> + BlockchainEvents<Block> + Send + Sync + 'static,
	<Client as ProvideRuntimeApi<Block>>::Api: MpcApi<Block>,
	B: Backend<Block> + 'static,
	Block: BlockT<Hash = H256> + Unpin,
	Block::Hash: Ord,
	N: Network<Block>,
	Ex: Spawn + 'static,
{
	let config = NodeConfig {
		duration: 1,
		threshold: 1,
		players: 3,
	};

	let local_peer_id = network.local_peer_id();
	let bridge = NetworkBridge::new(network, config.clone(), local_peer_id, &executor);
	let offchain_storage = backend.offchain_storage().expect("need offchain storage");

	let (tx, rx) = mpsc::unbounded();

	let streamer = client.clone().import_notification_stream().for_each(move |n| {
		let logs = n.header.digest().logs().iter();
		let block_number = n.header.number();
		if block_number == &2.into() {
			// temp workaround since cannot use polkadot js now
			let _ = tx.unbounded_send(MpcRequest::KeyGen(1234));
		} else if block_number == &5.into() {
			let _ = tx.unbounded_send(MpcRequest::SigGen(1235, 1234, vec![1u8]));
		}

		let arg = logs
			.filter_map(|l| l.try_to::<ConsensusLog>(OpaqueDigestItemId::Consensus(&MPC_ENGINE_ID)))
			.find_map(|l| match l {
				ConsensusLog::RequestForSig(req_id, pk_id, data) => Some(MpcRequest::SigGen(req_id, pk_id, data)),
				ConsensusLog::RequestForKey(id) => Some(MpcRequest::KeyGen(id)),
			});

		if let Some(arg) = arg {
			match arg {
				MpcRequest::SigGen(req_id, pk_id, mut data) => {
					let req = MpcRequest::SigGen(req_id, pk_id, data.clone());
					let _ = tx.unbounded_send(req);
					// if let Some(mut offchain_storage) = backend.offchain_storage() {
					// 	let key = get_storage_key(req_id, OffchainStorageType::Signature);
					// 	info!("key {:?} data {:?}", key, data);
					// 	let mut t = vec![1u8];
					// 	t.append(&mut data);
					// 	offchain_storage.set(STORAGE_PREFIX, &key, &t);
					// }
				}
				kg @ MpcRequest::KeyGen(_) => {
					let _ = tx.unbounded_send(kg);
				}
			}
		}

		ready(())
	});

	let kgw = KeyGenWork::new(client, config, bridge, offchain_storage, rx).map_err(|e| error!("Error {:?}", e));

	let worker = select(streamer, kgw).then(|_| ready(()));

	Ok(worker)
}
