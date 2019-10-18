extern crate serde;
extern crate unsigned_varint;

use keystore::KeyStorePtr;
use runtime_primitives::traits::Hash as THash;
use runtime_primitives::traits::{
  BlakeTwo256, Block as BlockT, Header, NumberFor, ProvideRuntimeApi,
};
use runtime_primitives::{
  generic::{self, BlockId},
  ApplyError, Justification,
};
//use runtime_primitives::{
//traits::{Block as BlockT, Hash as HashT, Header as HeaderT, ProvideRuntimeApi, DigestFor, },
//generic::BlockId,
//ApplyError,
//};
use consensus_common::evaluation;
//mod aux_schema;
use inherents::InherentIdentifier;
pub mod communication;
use communication::NetworkBridge;
use communication::PeerIdW;
use communication::TransactionSet;
use communication::QHB;
//use network::consensus_gossip::{self as network_gossip, MessageIntent, ValidatorContext};
use client::error::Error as ClientError;
use client::runtime_api::ConstructRuntimeApi;
use futures03::prelude::*;
use futures03::task::Poll;
use hex;
use network::PeerId; //config::Roles,
use runtime_primitives::traits::DigestFor;
use serde_json as json;
use std::{fmt::Debug, hash::Hash, marker::PhantomData, sync::Arc, time::Duration}; //time::Instant, thread,
use substrate_telemetry::{telemetry, CONSENSUS_INFO, CONSENSUS_WARN}; //CONSENSUS_TRATransactionCE, CONSENSUS_DEBUG,
use transaction_pool::txpool::{self, Pool as TransactionPool};

//use service::TelemetryOnConnect;
use badger::crypto::{PublicKey, SecretKey}; //PublicKeySet,PublicKeyShare,SecretKeyShare,Signature,
use client::backend::Backend;
use client::blockchain::HeaderBackend;
use client::CallExecutor;
use client::Client;
use std::str::FromStr;
//use client::blockchain::Backend;

use consensus_common::block_import::BlockImport;
use parity_codec::{Decode, Encode};

use consensus_common::import_queue::{
  BasicQueue, BoxBlockImport, BoxFinalityProofImport, BoxJustificationImport, CacheKeyId, Verifier,
};
use consensus_common::{self, BlockImportParams, BlockOrigin, ForkChoiceStrategy, SelectChain}; //Environment, Proposer,Error as ConsensusError,self,

use client::{
  //error::Result as CResult,
  backend::AuxStore,
  block_builder::api::BlockBuilder as BlockBuilderApi,
  blockchain::ProvideCache,
  //runtime_api::ApiExt,
  error,
};
use futures03::core_reexport::pin::Pin;

use crate::communication::Network;
use badger::ConsensusProtocol;
use fg_primitives::PublicKeySetWrap;
use fg_primitives::PublicKeyWrap;
use fg_primitives::SecretKeyShareWrap;
//use fg_primitives::SecretKeyWrap;
use std::path::PathBuf;
//use fg_primitives::AuthorityId;
use fg_primitives::AuthorityPair;
use substrate_badger_rapi::HbbftApi;
//use fg_primitives::AuthoritySignature;
use serde_json::Value;
use serde_json::Value::Number;
use serde_json::Value::Object;

use bincode;
use inherents::{InherentData, InherentDataProviders};
use std::fs::File;
//use std::fmt;
use futures03::future::Future;
use parking_lot::Mutex;
//use tokio_timer::Timeout;
use log::{debug, error, info, trace, warn};

pub use consensus_common::SyncOracle;

use substrate_primitives::{Blake2Hasher, ExecutionContext, H256};

pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"snakeshr";

pub type BadgerImportQueue<B> = BasicQueue<B>;

pub struct BadgerWorker<C, I, SO, Inbound, B: BlockT, N: Network<B>, A>
where
  A: txpool::ChainApi,
{
  pub client: Arc<C>,
  pub block_import: Arc<Mutex<I>>,
  pub network: NetworkBridge<B, N>,

  pub transaction_pool: Arc<TransactionPool<A>>,
  pub sync_oracle: SO,
  pub blocks_in: Inbound,
}

pub struct BadgerVerifier<C, Pub, Sig>
where
  Sig: std::marker::Send + std::marker::Sync,
  Pub: std::marker::Send + std::marker::Sync,
{
  client: Arc<C>,
  phantom: PhantomData<Pub>,
  phantom2: PhantomData<Sig>,
  inherent_data_providers: inherents::InherentDataProviders,
}

impl<C, Pub, Sig> BadgerVerifier<C, Pub, Sig>
where
  Sig: std::marker::Send + std::marker::Sync,
  Pub: std::marker::Send + std::marker::Sync,
{
  fn check_inherents<B: BlockT>(
    &self,
    _block: B,
    _block_id: BlockId<B>,
    _inherent_data: InherentData,
    _timestamp_now: u64,
  ) -> Result<(), String>
  where
    C: ProvideRuntimeApi,
    C::Api: BlockBuilderApi<B>,
  {
    //empty for now
    Ok(())
  }
}

impl<B: BlockT, C, Pub, Sig> Verifier<B> for BadgerVerifier<C, Pub, Sig>
where
  C: ProvideRuntimeApi + Send + Sync + client::backend::AuxStore + ProvideCache<B>,
  C::Api: BlockBuilderApi<B> + HbbftApi<B>,
  //DigestItemFor<B>: CompatibleDigestItem<P>,
  Pub: Send + Sync + Hash + Eq + Clone + Decode + Encode + Debug,
  Sig: Encode + Decode + Send + Sync,
{
  fn verify(
    &mut self,
    origin: BlockOrigin,
    header: B::Header,
    justification: Option<Justification>,
    body: Option<Vec<B::Extrinsic>>,
  ) -> Result<(BlockImportParams<B>, Option<Vec<(CacheKeyId, Vec<u8>)>>), String>
  {
    // dummy for the moment
    //let hash = header.hash();
    //let parent_hash = *header.parent_hash();
    let import_block = BlockImportParams {
      origin,
      header: header,
      post_digests: vec![],
      body,
      finalized: true,
      justification,
      auxiliary: Vec::new(),
      fork_choice: ForkChoiceStrategy::LongestChain,
    };
    info!("VERIFIER BADGER");
    Ok((import_block, None))
  }
}

/// Register the aura inherent data provider, if not registered already.
fn register_badger_inherent_data_provider(
  inherent_data_providers: &InherentDataProviders,
  _slot_duration: u64,
) -> Result<(), consensus_common::Error>
{
  if !inherent_data_providers.has_provider(&srml_timestamp::INHERENT_IDENTIFIER)
  {
    inherent_data_providers
      .register_provider(srml_timestamp::InherentDataProvider)
      .map_err(Into::into)
      .map_err(consensus_common::Error::InherentData)
  }
  else
  {
    Ok(())
  }
  //Ok(())
  /*	if !inherent_data_providers.has_provider(&hbbft::INHERENT_IDENTIFIER) {
      inherent_data_providers
          .register_provider(srml_aura::InherentDataProvider::new(slot_duration))
          .map_err(Into::into)
          .map_err(consensus_common::Error::InherentData)
  } else {
      Ok(())
  }*/
}

/// Start an import queue for the Badger consensus algorithm.
pub fn badger_import_queue<B, C, Pub, Sig>(
  block_import: BoxBlockImport<B>,
  justification_import: Option<BoxJustificationImport<B>>,
  finality_proof_import: Option<BoxFinalityProofImport<B>>,
  client: Arc<C>,
  inherent_data_providers: InherentDataProviders,
) -> Result<BadgerImportQueue<B>, consensus_common::Error>
where
  B: BlockT,
  C: 'static + ProvideRuntimeApi + ProvideCache<B> + Send + Sync + AuxStore,
  C::Api: BlockBuilderApi<B> + HbbftApi<B>,
  //DigestItemFor<B>: CompatibleDigestItem<P>,
  Pub: Clone + Eq + Send + Sync + Hash + Debug + Encode + Decode + 'static,
  Sig: Encode + Decode + Send + Sync + 'static,
{
  register_badger_inherent_data_provider(&inherent_data_providers, 1)?;
  //initialize_authorities_cache(&*client)?;

  let verifier = BadgerVerifier::<C, Pub, Sig> {
    client: client.clone(),
    inherent_data_providers,
    phantom: PhantomData,
    phantom2: PhantomData,
  };

  Ok(BasicQueue::new(
    verifier,
    block_import,
    justification_import,
    finality_proof_import,
  ))
}

use std::collections::BTreeMap;

//const REBROADCAST_AFTER: Duration = Duration::from_secs(60 * 5);
//const CATCH_UP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
//const CATCH_UP_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum number of rounds we are behind a peer before issuing a
/// catch up request.
//const CATCH_UP_THRESHOLD: u64 = 2;

//const KEEP_RECENT_ROUNDS: usize = 3;

//const BADGER_TOPIC: &str = "itsasnake";

/// Configuration for the Badger service.
#[derive(Clone)]
pub struct Config
{
  /// Some local identifier of the node.
  pub name: Option<String>,
  pub num_validators: usize,
  pub secret_key_share: Option<Arc<SecretKeyShareWrap>>,
  pub node_id: Arc<AuthorityPair>,
  pub public_key_set: Arc<PublicKeySetWrap>,
  pub batch_size: u32,
  pub initial_validators: BTreeMap<PeerIdW, PublicKey>,
  pub node_indices: BTreeMap<PeerIdW, usize>,
}
fn secret_share_from_string(st: &str) -> Result<SecretKeyShareWrap, Error>
{
  let data = hex::decode(st)?;
  match bincode::deserialize(&data)
  {
    Ok(val) => Ok(SecretKeyShareWrap { 0: val }),
    Err(_) => return Err(Error::Badger("secret key share binary invalid".to_string())),
  }
}

/*fn secret_from_string(st: &str) -> Result<SecretKeyWrap, Error>
{
  let data = hex::decode(st)?;
  match bincode::deserialize(&data)
  {
    Ok(val) => Ok(SecretKeyWrap { 0: val }),
    Err(_) => return Err(Error::Badger("secret key binary invalid".to_string())),
  }
}*/

impl Config
{
  fn name(&self) -> &str
  {
    self
      .name
      .as_ref()
      .map(|s| s.as_str())
      .unwrap_or("<unknown>")
  }
  pub fn from_json_file_with_name(path: PathBuf, name: &str) -> Result<Self, String>
  {
    let file = File::open(&path).map_err(|e| format!("Error opening config file: {}", e))?;
    let spec: serde_json::Value =
      json::from_reader(file).map_err(|e| format!("Error parsing spec file: {}", e))?;
    let nodedata = match &spec["nodes"]
    {
      Object(map) => match map.get(name)
      {
        Some(dat) => dat,
        None => return Err("Could not find node name".to_string()),
      },
      _ => return Err("Nodes object should be present".to_string()),
    };

    let ret = Config {
      name: Some(name.to_string()),
      num_validators: match &spec["num_validators"]
      {
        Number(x) => match x.as_u64()
        {
          Some(y) => y as usize,
          None => return Err("Invalid num_validators 1".to_string()),
        },
        Value::String(st) => match st.parse::<usize>()
        {
          Ok(v) => v,
          Err(_) => return Err("Invalid num_validators 2".to_string()),
        },
        _ => return Err("Invalid num_validators 3".to_string()),
      },
      secret_key_share: match &nodedata["secret_key_share"]
      {
        Value::String(st) => Some(Arc::new(match secret_share_from_string(&st)
        {
          Ok(val) => val,
          Err(_) => return Err("secret_key_share".to_string()),
        })),
        _ => None,
      },
      node_id: match &nodedata["node_id"]
      {
        Object(nid) =>
        {
          let pub_key: PublicKey = match &nid["pub"]
          {
            Value::String(st) =>
            {
              let data = match hex::decode(st)
              {
                Ok(val) => val,
                Err(_) => return Err("Hex error in pub".to_string()),
              };
              match bincode::deserialize(&data)
              {
                Ok(val) => val,
                Err(_) => return Err("public key binary invalid".to_string()),
              }
            }
            _ => return Err("pub key not string".to_string()),
          };
          let sec_key: SecretKey = match &nid["priv"]
          {
            Value::String(st) =>
            {
              let data = match hex::decode(st)
              {
                Ok(val) => val,
                Err(_) => return Err("Hex error in priv".to_string()),
              };
              match bincode::deserialize(&data)
              {
                Ok(val) => val,
                Err(_) => return Err("secret key binary invalid".to_string()),
              }
            }
            _ => return Err("priv key not string".to_string()),
          };
          Arc::new((pub_key, sec_key))
        }
        _ => return Err("node id not pub/priv object".to_string()),
      },
      public_key_set: match &spec["public_key_set"]
      {
        Value::String(st) =>
        {
          let data = match hex::decode(st)
          {
            Ok(val) => val,
            Err(_) => return Err("Hex error in public_key_set".to_string()),
          };
          match bincode::deserialize(&data)
          {
            Ok(val) => Arc::new(PublicKeySetWrap { 0: val }),
            Err(_) => return Err("public key set binary invalid".to_string()),
          }
        }
        _ => return Err("pub key set not string".to_string()),
      },
      batch_size: match &spec["batch_size"]
      {
        Number(x) => match x.as_u64()
        {
          Some(y) => y as u32,
          None => return Err("Invalid batch_size".to_string()),
        },
        Value::String(st) => match st.parse::<u32>()
        {
          Ok(val) => val,
          Err(_) => return Err("batch_size parsing error".to_string()),
        },
        _ => return Err("Invalid batch_size".to_string()),
      },

      initial_validators: match &spec["initial_validators"]
      {
        Object(dict) =>
        {
          let mut ret = BTreeMap::<PeerIdW, PublicKey>::new();
          for (k, v) in dict.iter()
          {
            let peer = match PeerId::from_str(k)
            {
              Ok(val) => val,
              Err(_) => return Err("Invalid PeerId".to_string()),
            };
            let data = match hex::decode(v.as_str().unwrap())
            {
              Ok(val) => val,
              Err(_) => return Err("Hex error in pubkey".to_string()),
            };
            let pubkey: PublicKeyWrap = match bincode::deserialize(&data)
            {
              Ok(val) => val,
              Err(_) => return Err("public key binary invalid".to_string()),
            };
            info!("JSON! {:?} {:?}", &peer, &pubkey.0);
            //		let cpeer=peer.clone();
            ret.insert(PeerIdW { 0: peer }, pubkey.0);
          }

          ret
        }
        _ => return Err("Invalid initial_validators, should be object".to_string()),
      },
      node_indices: match &spec["node_indices"]
      {
        Object(dict) =>
        {
          let mut ret = BTreeMap::<PeerIdW, usize>::new();
          for (k, v) in dict.iter()
          {
            let peer = match PeerId::from_str(k)
            {
              Ok(val) => val,
              Err(_) => return Err("Invalid PeerId".to_string()),
            };
            let numb = match &v
            {
              Number(x) => match x.as_u64()
              {
                Some(y) => y as usize,
                None => return Err("Invalid num_validators 1".to_string()),
              },
              Value::String(st) => match st.parse::<usize>()
              {
                Ok(v) => v,
                Err(_) => return Err("Invalid num_validators 2".to_string()),
              },
              _ => return Err("Invalid num_validators 3".to_string()),
            };

            ret.insert(PeerIdW { 0: peer }, numb);
          }

          ret
        }
        _ => return Err("Invalid initial_validators, should be object".to_string()),
      },
    };
    Ok(ret)
  }
}

/// Errors that can occur while voting in BADGER.
#[derive(Debug)]
pub enum Error
{
  /// An error within badger.
  Badger(String),
  /// A network error.
  Network(String),
  /// A blockchain error.
  Blockchain(String),
  /// Could not complete a round on disk.
  Client(ClientError),
  /// An invariant has been violated (e.g. not finalizing pending change blocks in-order)
  Safety(String),
  /// A timer failed to fire.
  Timer(tokio_timer::Error),
}

impl From<hex::FromHexError> for Error
{
  fn from(e: hex::FromHexError) -> Self
  {
    Error::Badger(e.to_string())
  }
}
impl From<ClientError> for Error
{
  fn from(e: ClientError) -> Self
  {
    Error::Client(e)
  }
}

/// Something which can determine if a block is known.
pub trait BlockStatus<Block: BlockT>
{
  /// Return `Ok(Some(number))` or `Ok(None)` depending on whether the block
  /// is definitely known and has been imported.
  /// If an unexpected error occurs, return that.
  fn block_number(&self, hash: Block::Hash) -> Result<Option<NumberFor<Block>>, Error>;
}
pub trait BlockNumberOps:
  std::fmt::Debug
  + std::cmp::Ord
  + std::ops::Add<Output = Self>
  + std::ops::Sub<Output = Self>
  + num::One
  + num::Zero
  + num::AsPrimitive<usize>
{
}

impl<T> BlockNumberOps for T
where
  T: std::fmt::Debug,
  T: std::cmp::Ord,
  T: std::ops::Add<Output = Self>,
  T: std::ops::Sub<Output = Self>,
  T: num::One,
  T: num::Zero,
  T: num::AsPrimitive<usize>,
{
}

impl<B, E, Block: BlockT<Hash = H256>, RA> BlockStatus<Block> for Arc<Client<B, E, Block, RA>>
where
  B: Backend<Block, Blake2Hasher>,
  E: CallExecutor<Block, Blake2Hasher> + Send + Sync,
  RA: Send + Sync,
  NumberFor<Block>: BlockNumberOps,
{
  fn block_number(&self, hash: Block::Hash) -> Result<Option<NumberFor<Block>>, Error>
  {
    self
      .block_number_from_id(&BlockId::Hash(hash))
      .map_err(|e| Error::Blockchain(format!("{:?}", e)))
  }
}

/*
pub struct BadgerBlockImport<B, E, Block: BlockT<Hash=H256>, RA, PRA, SC> {
    inner: Arc<Client<B, E, Block, RA>>,
    select_chain: SC,
    //authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
    //send_voter_commands: mpsc::UnboundedSender<VoterCommand<Block::Hash, NumberFor<Block>>>,
    consensus_changes: SharedConsensusChanges<Block::Hash, NumberFor<Block>>,
    api: Arc<PRA>,
}

impl<B, E, Block: BlockT<Hash=H256>, RA, PRA, SC: Clone> Clone for
    GrandpaBlockImport<B, E, Block, RA, PRA, SC>
{
    fn clone(&self) -> Self {
        GrandpaBlockImport {
            inner: self.inner.clone(),
            select_chain: self.select_chain.clone(),
            authority_set: self.authority_set.clone(),
            send_voter_commands: self.send_voter_commands.clone(),
            consensus_changes: self.consensus_changes.clone(),
            api: self.api.clone(),
        }
    }
}

impl<B, E, Block: BlockT<Hash=H256>, RA, PRA, SC> JustificationImport<Block>
    for GrandpaBlockImport<B, E, Block, RA, PRA, SC> where
        NumberFor<Block>: grandpa::BlockNumberOps,
        B: Backend<Block, Blake2Hasher> + 'static,
        E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
        DigestFor<Block>: Encode,
        RA: Send + Sync,
        PRA: ProvideRuntimeApi,
        PRA::Api: GrandpaApi<Block>,
        SC: SelectChain<Block>,
{
    type Error = ConsensusError;

    fn on_start(&mut self) -> Vec<(Block::Hash, NumberFor<Block>)> {
        let mut out = Vec::new();
        let chain_info = self.inner.info().chain;

        // request justifications for all pending changes for which change blocks have already been imported
        let authorities = self.authority_set.inner().read();
        for pending_change in authorities.pending_changes() {
            if pending_change.delay_kind == DelayKind::Finalized &&
                pending_change.effective_number() > chain_info.finalized_number &&
                pending_change.effective_number() <= chain_info.best_number
            {
                let effective_block_hash = self.select_chain.finality_target(
                    pending_change.canon_hash,
                    Some(pending_change.effective_number()),
                );

                if let Ok(Some(hash)) = effective_block_hash {
                    if let Ok(Some(header)) = self.inner.header(&BlockId::Hash(hash)) {
                        if *header.number() == pending_change.effective_number() {
                            out.push((header.hash(), *header.number()));
                        }
                    }
                }
            }
        }

        out
    }

    fn import_justification(
        &mut self,
        hash: Block::Hash,
        number: NumberFor<Block>,
        justification: Justification,
    ) -> Result<(), Self::Error> {
        self.import_justification(hash, number, justification, false)
    }
}



*/

//pub struct LinkHalf<B, E, Block: BlockT<Hash=H256>, RA, SC> {
//	client: Arc<Client<B, E, Block, RA>>,

//persistent_data: PersistentData<Block>,
//voter_commands_rx: mpsc::UnboundedReceiver<VoterCommand<Block::Hash, NumberFor<Block>>>,
//}

/// Make block importer and link half necessary to tie the background voter
/// to it.
/*pub fn block_import<B, E, Block: BlockT<Hash=H256>, RA, PRA, SC>(
    client: Arc<Client<B, E, Block, RA>>,
    api: Arc<PRA>,
    select_chain: SC,
) -> Result<BadgerBlockImport<B, E, Block, RA, PRA, SC>, ClientError>
where
    B: Backend<Block, Blake2Hasher> + 'static,
    E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
    RA: Send + Sync,
    PRA: ProvideRuntimeApi,
    PRA::Api: HbbftApi<Block>,
    SC: SelectChain<Block>,
{


    let chain_info = client.info();
    let genesis_hash = chain_info.chain.genesis_hash;

    /* let persistent_data = aux_schema::load_persistent(
        #[allow(deprecated)]
        &**client.backend(),
        genesis_hash,
        <NumberFor<Block>>::zero(),
        || {
            let genesis_authorities = api.runtime_api()
                .badger_authorities(&BlockId::number(Zero::zero()))?;
            telemetry!(CONSENSUS_DEBUG; "afg.loading_authorities";
                "authorities_len" => ?genesis_authorities.len()
            );
            Ok(genesis_authorities)
        }
    )?; */

    let (voter_commands_tx, voter_commands_rx) = mpsc::unbounded();

    Ok((
        BadgerBlockImport::new(
            client.clone(),
            select_chain.clone(),
            persistent_data.authority_set.clone(),
            voter_commands_tx,
            persistent_data.consensus_changes.clone(),
            api,
        ),
        LinkHalf {
            client,
            select_chain,
            persistent_data,
            voter_commands_rx,
        },
    ))
} */
use crate::communication::SendOut;
fn global_communication<Block: BlockT<Hash = H256>, B, E, N, RA>(
  client: &Arc<Client<B, E, Block, RA>>,
  network: NetworkBridge<Block, N>,
) -> (
  impl Stream<Item = <QHB as ConsensusProtocol>::Output>,
  impl SendOut,
)
where
  B: Backend<Block, Blake2Hasher>,
  E: CallExecutor<Block, Blake2Hasher> + Send + Sync,
  N: Network<Block>,
  RA: Send + Sync,
  NumberFor<Block>: BlockNumberOps,
{
  let is_voter = network.is_validator();

  // verification stream
  let (global_in, global_out) = network.global_communication(
    //	voters.clone(),
    is_voter,
  );

  // block commit and catch up messages until relevant blocks are imported.
  /*let global_in = UntilGlobalMessageBlocksImported::new(
      client.import_notification_stream(),
      client.clone(),
      global_in,
  );*/ //later

  (global_in, global_out)
}

/// Parameters used to run Honeyed Badger.
pub struct BadgerStartParams<Block: BlockT<Hash = H256>, N: Network<Block>, X>
{
  /// Configuration for the Badger service.
  pub config: Config,
  /// A link to the block import worker.
  //pub link: LinkHalf<B, E, Block, RA, SC>,
  /// The Network instance.
  pub network: N,
  /// The inherent data providers.
  pub inherent_data_providers: InherentDataProviders,
  /// Handle to a future that will resolve on exit.
  pub on_exit: X,
  /// If supplied, can be used to hook on telemetry connection established events.
  //	pub telemetry_on_connect: Option<TelemetryOnConnect>,
  ph: PhantomData<Block>,
}

pub struct TxStream<A, B, E, RA, Block: BlockT<Hash = H256>>
where
  A: txpool::ChainApi,
  B: client::backend::Backend<Block, Blake2Hasher> + Send + Sync + 'static,
  E: CallExecutor<Block, Blake2Hasher> + Send + Sync + Clone + 'static,
  RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
{
  pub transaction_pool: Arc<TransactionPool<A>>,
  pub client: Arc<Client<B, E, Block, RA>>,
}

impl<A, B, E, RA, Block: BlockT<Hash = H256>> Stream for TxStream<A, B, E, RA, Block>
where
  A: txpool::ChainApi,
  <A as txpool::ChainApi>::Block: BlockT<Hash = H256>,
  B: client::backend::Backend<Block, Blake2Hasher> + Send + Sync + 'static,
  E: CallExecutor<Block, Blake2Hasher> + Send + Sync + Clone + 'static,
  RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
{
  type Item = TransactionSet;
  fn poll_next(self: Pin<&mut Self>, _cx: &mut futures03::task::Context)
    -> Poll<Option<Self::Item>>
  {
    trace!("BADgER! Polled stream!");
    let pending_iterator = self.transaction_pool.ready();
    let batch: Vec<_> = pending_iterator.map(|a| a.data.encode()).collect();
    let pending_iterator = self.transaction_pool.ready();
    let tags: Vec<_> = pending_iterator
      .flat_map(|a| a.provides.clone().into_iter())
      .collect();
    let pending_iterator = self.transaction_pool.ready();
    let hashes: Vec<_> = pending_iterator.map(|a| a.hash.clone()).collect();
    if batch.len() == 0
    {
      return Poll::Pending;
    }
    info!("BADgER! Ready stream!");
    let best_block_hash = self.client.info().chain.best_hash;
    self
      .transaction_pool
      .prune_tags(&generic::BlockId::hash(best_block_hash), tags, hashes);
    Poll::Ready(Some(batch))
  }
}

pub struct BadgerProposerWorker<S, Block: BlockT<Hash = H256>, I, B, E, RA, SC>
where
  S: Stream<Item = <QHB as ConsensusProtocol>::Output>,
  //TF: Sink<TransactionSet>+Unpin,
  //A: txpool::ChainApi,
  B: client::backend::Backend<Block, Blake2Hasher> + Send + Sync + 'static,
  E: CallExecutor<Block, Blake2Hasher> + Send + Sync + Clone + 'static,
  RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
  //N : Network<Block> + Send + Sync + 'static,
  NumberFor<Block>: BlockNumberOps,
  SC: SelectChain<Block> + Clone,
  I: BlockImport<Block>,
{
  pub block_out: S,
  //pub transaction_in: TF,
  //pub transaction_out: TxStream<A>,//Arc<TransactionPool<A>>,
  //pub network: N,
  pub client: Arc<Client<B, E, Block, RA>>,
  pub block_import: Arc<Mutex<I>>,
  pub inherent_data_providers: InherentDataProviders,
  pub select_chain: SC,
  phb: PhantomData<Block>,
}

/*
impl<S: Stream<Item = <QHB as ConsensusProtocol>::Output> +'static ,Block:BlockT<Hash=H256>,I:BlockImport<Block>+'static,B,E,RA:'static,SC:'static>   BadgerProposerWorker<S,Block,I,B,E,RA,SC>
where
NumberFor<Block>: BlockNumberOps,
Client<B, E, Block, RA>: ProvideRuntimeApi,
<Client<B, E, Block, RA> as ProvideRuntimeApi>::Api: BlockBuilderApi<Block>,
B: client::backend::Backend<Block, Blake2Hasher> + Send + Sync + 'static,
E: CallExecutor<Block, Blake2Hasher> + Send + Sync + Clone + 'static,
RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>+ Send + Sync,
SC:SelectChain<Block> + Clone,
S: Unpin
{



   pub fn  get_aggregate(mut self) -> impl Future
   {


   }

}
*/
/*impl<D:ConsensusProtocol<NodeId=PeerIdW>,S: Stream<Item = D::Output>  ,N,Block:BlockT,TF: Sink<TransactionSet>+Unpin,C,A: txpool::ChainApi,I:BlockImport<Block>>
  Future on BadgerProposerWorker<D,S,N,Block,TF,C,A,I>
{
    type Output=();
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output>
    {

    }
}*/
//use futures_timer::Delay;
use futures_timer::Interval;
/// Run a HBBFT churn as a task. Provide configuration and a link to a
/// block import worker that has already been instantiated with `block_import`.
pub fn run_honey_badger<B, E, Block: BlockT<Hash = H256>, N, RA, SC, X, I, A>(
  client: Arc<Client<B, E, Block, RA>>,
  t_pool: Arc<TransactionPool<A>>,
  config: Config,
  network: N,
  on_exit: X,
  block_import: Arc<Mutex<I>>,
  inherent_data_providers: InherentDataProviders,
  selch: SC,
  keystore: KeyStorePtr,
) -> ::client::error::Result<impl Future<Output = ()> + Send + Unpin>
where
  Block::Hash: Ord,
  B: Backend<Block, Blake2Hasher> + 'static,
  E: CallExecutor<Block, Blake2Hasher> + Send + Sync + 'static + Clone,
  N: Network<Block> + Send + Sync + Unpin,
  N::In: Send,
  SC: SelectChain<Block> + 'static + Unpin,
  NumberFor<Block>: BlockNumberOps,
  DigestFor<Block>: Encode,
  RA: Send + Sync + 'static,
  X: futures03::future::Future<Output = ()> + Send + Unpin,
  A: txpool::ChainApi + 'static,
  <A as txpool::ChainApi>::Block: BlockT<Hash = H256>,
  I: BlockImport<Block> + Send + Sync + 'static,
  RA: ConstructRuntimeApi<Block, Client<B, E, Block, RA>>,
  <Client<B, E, Block, RA> as ProvideRuntimeApi>::Api: BlockBuilderApi<Block>,
{
  let (network_bridge, network_startup) = NetworkBridge::new(network, config.clone());

  //let PersistentData { authority_set, set_state, consensus_changes } = persistent_data;

  //	register_finality_tracker_inherent_data_provider(client.clone(), &inherent_data_providers)?;
  let (blk_out, tx_in) = global_communication(&client, network_bridge);
  let txcopy = Arc::new(parking_lot::RwLock::new(tx_in));
  let tx_in_arc = txcopy.clone();

  let tx_out = TxStream {
    transaction_pool: t_pool.clone(),
    client: client.clone(),
  };
  let sender = tx_out.for_each(move |data: std::vec::Vec<std::vec::Vec<u8>>| {
    {
      let mut lock = tx_in_arc.write();
      match lock.send_out(data)
      {
        Ok(_) =>
        {}
        Err(_) =>
        {
          debug!("Well, this is weird");
        }
      }
    }
    future::ready(())
  });
  let cclient = client.clone();
  let cblock_import = block_import.clone();
  let ping_sel = selch.clone();
  let receiver = blk_out.for_each(move |batch| {
    info!("[[[[[[[");
    let inherent_digests = generic::Digest { logs: vec![] };
    info!(
      "Processing batch with epoch {:?} of {:?} transactions into blocks",
      batch.epoch(),
      batch.len()
    );
    let chain_head = match selch.best_chain()
    {
      Ok(x) => x,
      Err(e) =>
      {
        warn!(target: "formation", "Unable to author block, no best block header: {:?}", e);
        return future::ready(());
      }
    };
    let parent_hash = chain_head.hash();
    let mut pnumber = *chain_head.number();
    let mut parent_id = BlockId::hash(parent_hash);
    let mut block_builder = match cclient.new_block_at(&parent_id, inherent_digests.clone())
    {
      Ok(val) => val,
      Err(_) =>
      {
        warn!("Error in new_block_at");
        return future::ready(());
      }
    };

    // proceed with transactions
    let mut is_first = true;
    //let mut skipped = 0;
    info!(
      "Attempting to push transactions from the batch. {}",
      batch.len()
    );

    for pending in batch.iter()
    {
      let data: Result<<Block as BlockT>::Extrinsic, _> = Decode::decode(&mut pending.as_slice());
      //   <Transaction<ExHash<A>,<Block as BlockT>::Extrinsic> as Decode>::decode(&mut pending.as_slice());
      let data = match data
      {
        Ok(val) => val,
        Err(_) =>
        {
          info!("Data decoding error");
          continue;
        }
      };
      //a.data.encode()
      info!("[{:?}] Pushing to the block.", pending);
      info!("[{:?}] Data ", &data);
      match client::block_builder::BlockBuilder::push(&mut block_builder, data.clone())
      {
        Ok(()) =>
        {
          debug!("[{:?}] bytes Pushed to the block.", pending.len());
        }
        Err(error::Error::ApplyExtrinsicFailed(ApplyError::BadState)) =>
        {
          if is_first
          {
            debug!(
              "[{:?}] Invalid transaction: FullBlock on empty block",
              pending.len()
            );
          //	unqueue_invalid.push(pending.hash.clone());
          }
          else
          {
            debug!("Block is full, proceed with proposing.");
            let block = match block_builder.bake()
            {
              Ok(val) => val,
              Err(e) =>
              {
                warn!("Block baking error {:?}", e);
                return future::ready(());
              }
            };
            info!(
              "Prepared block for proposing at {} [hash: {:?}; parent_hash: {}; extrinsics: [{}]]",
              block.header().number(),
              <Block as BlockT>::Hash::from(block.header().hash()),
              block.header().parent_hash(),
              block
                .extrinsics()
                .iter()
                .map(|xt| format!("{}", BlakeTwo256::hash_of(xt)))
                .collect::<Vec<_>>()
                .join(", ")
            );
            telemetry!(CONSENSUS_INFO; "prepared_block_for_proposing";
            "number" => ?block.header().number(),
            "hash" => ?<Block as BlockT>::Hash::from(block.header().hash()),
            );
            if Decode::decode(&mut block.encode().as_slice()).as_ref() != Ok(&block)
            {
              error!("Failed to verify block encoding/decoding");
            }

            if let Err(err) = evaluation::evaluate_initial(&block, &parent_hash, pnumber)
            {
              error!("Failed to evaluate authored block: {:?}", err);
            }
            let (header, body) = block.deconstruct();

            let header_num = header.number().clone();
            let mut parent_hash; //= header.parent_hash().clone();

            // sign the pre-sealed hash of the block and then
            // add it to a digest item.
            let header_hash = header.hash();

            let import_block: BlockImportParams<Block> = BlockImportParams {
              origin: BlockOrigin::Own,
              header,
              justification: None,
              post_digests: vec![],
              body: Some(body),
              finalized: true,
              auxiliary: Vec::new(),
              fork_choice: ForkChoiceStrategy::LongestChain,
            };

            info!(
              "Pre-sealed block for proposal at {}. Hash now {:?}, previously {:?}.",
              header_num,
              import_block.post_header().hash(),
              header_hash
            );
            telemetry!(CONSENSUS_INFO; "badger.pre_sealed_block";
              "header_num" => ?header_num,
              "hash_now" => ?import_block.post_header().hash(),
              "hash_previously" => ?header_hash
            );
            parent_hash = import_block.post_header().hash();
            pnumber = *import_block.post_header().number();
            parent_id = BlockId::hash(parent_hash);
            // go on to next block
            {
              let eh = import_block.header.parent_hash().clone();
              if let Err(e) = cblock_import
                .lock()
                .import_block(import_block, Default::default())
              {
                warn!(target: "badger", "Error with block built on {:?}: {:?}",eh, e);
                telemetry!(CONSENSUS_WARN; "mushroom.err_with_block_built_on";
							"hash" => ?eh, "err" => ?e);
              }
              block_builder = cclient
                .new_block_at(&parent_id, inherent_digests.clone())
                .unwrap();
              is_first = true;
              continue;
            }
          }
        }
        Err(e) =>
        {
          debug!("[{:?}] Invalid transaction: {}", pending.len(), e);
          //unqueue_invalid.push(pending.hash.clone());
        }
      }

      is_first = false;
    }

    if !is_first
    {
      info!("BADger: importing block");
      {
        debug!("Block is done, proceed with proposing.");
        let block = match block_builder.bake()
        {
          Ok(val) => val,
          Err(e) =>
          {
            warn!("Block baking error {:?}", e);
            return future::ready(());
          }
        };
        info!(
          "Prepared block for proposing at {} [hash: {:?}; parent_hash: {}; extrinsics: [{}]]",
          block.header().number(),
          <Block as BlockT>::Hash::from(block.header().hash()),
          block.header().parent_hash(),
          block
            .extrinsics()
            .iter()
            .map(|xt| format!("{}", BlakeTwo256::hash_of(xt)))
            .collect::<Vec<_>>()
            .join(", ")
        );
        telemetry!(CONSENSUS_INFO; "prepared_block_for_proposing";
        "number" => ?block.header().number(),
        "hash" => ?<Block as BlockT>::Hash::from(block.header().hash()),
        );
        if Decode::decode(&mut block.encode().as_slice()).as_ref() != Ok(&block)
        {
          error!("Failed to verify block encoding/decoding");
        }

        if let Err(err) = evaluation::evaluate_initial(&block, &parent_hash, pnumber)
        {
          error!("Failed to evaluate authored block: {:?}", err);
        }
        let (header, body) = block.deconstruct();

        let header_num = header.number().clone();
        let mut parent_hash = header.parent_hash().clone();

        // sign the pre-sealed hash of the block and then
        // add it to a digest item.
        let header_hash = header.hash();

        let import_block: BlockImportParams<Block> = BlockImportParams {
          origin: BlockOrigin::Own,
          header,
          justification: None,
          post_digests: vec![],
          body: Some(body),
          finalized: true,
          auxiliary: Vec::new(),
          fork_choice: ForkChoiceStrategy::LongestChain,
        };

        info!(
          "Pre-sealed block for proposal at {}. Hash now {:?}, previously {:?}.",
          header_num,
          import_block.post_header().hash(),
          header_hash
        );
        telemetry!(CONSENSUS_INFO; "badger.pre_sealed_block";
          "header_num" => ?header_num,
          "hash_now" => ?import_block.post_header().hash(),
          "hash_previously" => ?header_hash
        );
        parent_hash = import_block.post_header().hash();
        pnumber = *import_block.post_header().number();
        parent_id = BlockId::hash(parent_hash);
        // go on to next block
        {
          let eh = import_block.header.parent_hash().clone();
          if let Err(e) = cblock_import
            .lock()
            .import_block(import_block, Default::default())
          {
            warn!(target: "badger", "Error with block built on {:?}: {:?}",eh, e);
            telemetry!(CONSENSUS_WARN; "mushroom.err_with_block_built_on";
							"hash" => ?eh, "err" => ?e);
          }
        }
      }
    }
    info!("[[[[[[[--]]]]]]]");

    future::ready(())
  });

  //.map(|_| ()).map_err(|e|
  //    {
  //			warn!("BADGER failed: {:?}", e);
  //			telemetry!(CONSENSUS_WARN; "afg.badger_failed"; "e" => ?e);
  //		}) ;

  let with_start = network_startup.then(move |()| futures03::future::join(sender, receiver));
  let ping_client = client.clone();
  // Delay::new(Duration::from_secs(1)).then(|_| {
  let ping = Interval::new(Duration::from_millis(11500)).for_each(move |_| {
    //put inherents here for now
    let mut chain_head = match ping_sel.best_chain()
    {
      Ok(x) => x,
      Err(e) =>
      {
        warn!(target: "formation", "Unable to author block, no best block header: {:?}", e);
        return future::ready(());
      }
    };
    let mut parent_hash = chain_head.hash();
    let mut pnumber = *chain_head.number();
    let mut parent_id = BlockId::hash(parent_hash);

    let inherent_data = match inherent_data_providers.create_inherent_data()
    {
      Ok(id) => id,
      Err(err) => return future::ready(()), //future::err(err),
    };
    //empty for now?

    // We don't check the API versions any further here since the dispatch compatibility
    // check should be enough.
    // do this only once?
    let inh = ping_client.runtime_api().inherent_extrinsics_with_context(
      &parent_id,
      ExecutionContext::BlockConstruction,
      inherent_data,
    );
    if let Ok(res) = inh
    {
      let mut lock = txcopy.write();
      info!("This many INHERS {:?}", res.len());
      for extrinsic in res
      {
        info!("INHER {:?}", &extrinsic);
        match lock.send_out(vec![extrinsic.encode().into_iter().collect()])
        {
          Ok(_) =>
          {}
          Err(_) =>
          {
            warn!("Error in ping sendout");
            return future::ready(());
          }
        }
      }
    }
    else
    {
      info!("Inherent panic {:?}", inh);
    }
    info!("Ping done");
    future::ready(())
  });

  //let jn = ping.merge(t_pool.clone().import_notification_stream());
  let pinged = futures03::future::select(with_start, ping);
  // Make sure that `telemetry_task` doesn't accidentally finish and kill grandpa.

  Ok(
    futures03::future::select(
      on_exit.then(|_| {
        info!("READY");
        future::ready(())
      }),
      pinged,
    )
    .then(|_| future::ready(())),
  )
  /*let ping_lesser = Interval::new(Duration::from_millis(1000)).for_each(move |_| {
    info!("ping");
        let mut chain_head = match ping_sel.best_chain()
    {
      Ok(x) => x,
      Err(e) =>
      {
        warn!(target: "formation", "Unable to author block, no best block header: {:?}", e);
        return future::ready(());
      }
    };
  let mut parent_hash = chain_head.hash();
    let mut pnumber = *chain_head.number();
    let mut parent_id = BlockId::hash(parent_hash);

    let inherent_data = match inherent_data_providers.create_inherent_data()
    {
      Ok(id) => id,
      Err(err) => return future::ready(()), //future::err(err),
    };
  let inh = ping_client.runtime_api().inherent_extrinsics_with_context(
      &parent_id,
      ExecutionContext::BlockConstruction,
      inherent_data,
    );
      info!("ping end");
    future::ready(())
    });

  let ready_on_exit= on_exit.then(|_| {
        info!("READY");
        future::ready(())
      });
  Ok(  futures03::future::select( ping_lesser,ready_on_exit)  .then(|_| future::ready(())))*/
}
