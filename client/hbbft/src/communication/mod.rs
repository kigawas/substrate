use badger::dynamic_honey_badger::DynamicHoneyBadger;
use badger::queueing_honey_badger::QueueingHoneyBadger;
use badger::sender_queue::{Message as BMessage, SenderQueue};
use badger::sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, PubKeyMap, SyncKeyGen};//AckFault
use keystore::KeyStorePtr;
use runtime_primitives::generic::BlockId;
use sc_api::{Backend,AuxStore};
//use runtime_primitives::app_crypto::RuntimeAppPublic;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
//use std::convert::TryInto;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use gossip::BadgerJustification;
use runtime_primitives::Justification;

use client::{
	CallExecutor, Client, well_known_cache_keys
};
use gossip::{BadgerFullJustification,BadgerAuthCommit};
use substrate_primitives::{Blake2Hasher,};
use hash_db::Hasher;
use badger_primitives::AuthoritySignature;
//use badger::dynamic_honey_badger::KeyGenMessage::Ack;
use crate::aux_store::BadgerPersistentData;
use badger::crypto::{PublicKey, PublicKeySet, SecretKey, SecretKeyShare}; //PublicKeyShare, Signature
use badger::{dynamic_honey_badger::Change, ConsensusProtocol, CpStep, NetworkInfo, Target};
use futures03::channel::{mpsc, oneshot};
use futures03::prelude::*;
use futures03::{task::Context, task::Poll};
//use hex_fmt::HexFmt;
use badger::honey_badger::EncryptionSchedule;
use log::{debug, info, trace, warn};
use parity_codec::{Decode, Encode};
use parking_lot::RwLock;
use rand::{rngs::OsRng, Rng};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
//
//};//
pub use badger_primitives::HBBFT_ENGINE_ID;
use badger_primitives::{AuthorityId, AuthorityList, AuthorityPair};
use gossip::{BadgeredMessage, GossipMessage, Peers, SAction, SessionData, SessionMessage};
use network::config::Roles;
use network::consensus_gossip::MessageIntent;
use network::consensus_gossip::ValidatorContext;
use network::PeerId;
use network::{consensus_gossip as network_gossip, NetworkService,ReputationChange};
use network_gossip::ConsensusMessage;
//use runtime_primitives::app_crypto::hbbft_thresh::Public as bPublic;
use runtime_primitives::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use substrate_primitives::crypto::Pair;
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};

pub mod gossip;

use crate::Error;

mod peerid;
pub use peerid::PeerIdW;
pub const MAX_DELAYED_JUSTIFICATIONS:u64=10 ;
//use badger_primitives::NodeId;

//use badger::{SourcedMessage as BSM,  TargetedMessage};
pub type NodeId = PeerIdW; //session index? -> might be difficult. but it looks like nodeId for observer does not matter?  but it does for restarts

pub trait Network<Block: BlockT>: Clone + Send + 'static
{
  type In: Stream<Item = network_gossip::TopicNotification>;

  fn messages_for(&self, topic: Block::Hash) -> Self::In;

  fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<Block>>);

  fn gossip_message(&self, topic: Block::Hash, data: Vec<u8>, force: bool);

  fn register_gossip_message(&self, topic: Block::Hash, data: Vec<u8>);

  fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>);

  fn report(&self, who: network::PeerId, cost_benefit: i32);

  fn announce(&self, block: Block::Hash, associated_data: Vec<u8>);

  fn local_id(&self) -> PeerId;
}

pub fn badger_topic<B: BlockT>() -> B::Hash
{
  <<B::Header as HeaderT>::Hashing as HashT>::hash(format!("badger-mushroom").as_bytes())
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum LocalTarget
{
  /// The message must be sent to the node with the given ID.
  Nodes(BTreeSet<NodeId>),
  /// The message must be sent to all remote nodes except the passed nodes.
  /// Useful for sending messages to observer nodes that aren't
  /// present in a node's `all_ids()` list.
  AllExcept(BTreeSet<NodeId>),
}

impl From<Target<NodeId>> for LocalTarget
{
  fn from(t: Target<NodeId>) -> Self
  {
    match t
    {
      // Target::All => LocalTarget::All,
      Target::Nodes(n) => LocalTarget::Nodes(n),
      Target::AllExcept(set) => LocalTarget::AllExcept(set),
    }
  }
}

impl Into<Target<NodeId>> for LocalTarget
{
  fn into(self) -> Target<NodeId>
  {
    match self
    {
      // LocalTarget::All => Target::All,
      LocalTarget::Nodes(n) => Target::Nodes(n),
      LocalTarget::AllExcept(set) => Target::AllExcept(set),
    }
  }
}

#[derive(Eq, PartialEq, Debug, Encode, Decode)]
pub struct SourcedMessage<D: ConsensusProtocol>
where
  D::NodeId: Encode + Decode + Clone,
{
  sender_id: D::NodeId,
  target: LocalTarget,
  message: Vec<u8>,
}

impl rand::distributions::Distribution<PeerIdW> for rand::distributions::Standard
{
  fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> PeerIdW
  {
    PeerIdW(PeerId::random())
  }
}

impl rand::distributions::Distribution<PeerIdW> for PeerIdW
{
  fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> PeerIdW
  {
    PeerIdW(PeerId::random())
  }
}

pub type BadgerTransaction = Vec<u8>;
pub type QHB = SenderQueue<QueueingHoneyBadger<BadgerTransaction, NodeId, Vec<BadgerTransaction>>>;

pub struct BadgerNode<B: BlockT, D>
where
  D: ConsensusProtocol<NodeId = NodeId>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
{
  //id: PeerId,
  node_id: NodeId,
  algo: D,
  main_rng: OsRng,

  //peers: Peers,
  //authorities: Vec<AuthorityId>,
  //config: crate::Config,
  //next_rebroadcast: Instant,
  /// Incoming messages from other nodes that this node has not yet handled.
  // in_queue: VecDeque<SourcedMessage<D>>,
  /// Outgoing messages to other nodes.
  pub out_queue: VecDeque<SourcedMessage<D>>,
  /// The values this node has output so far, with timestamps.
  pub outputs: VecDeque<D::Output>,
  _block: PhantomData<B>,
}

pub enum KeyGenStep
{
  Init,
  PartGenerated,
  PartSent,
  ProcessingAcks,
  Done,
}

pub struct KeyGenState
{
  pub is_observer: bool,
  pub keygen: SyncKeyGen<NodeId>,
  pub part: Option<Part>,
  pub threshold: usize,
  pub expected_parts: VecDeque<NodeId>,
  pub expected_acks: VecDeque<(NodeId, VecDeque<NodeId>)>,
  pub buffered_messages: Vec<(NodeId, SyncKeyGenMessage)>,
  pub is_done: bool,
  //pub ack: Option<(NodeId,AckOutcome)>,
  //pub step:KeyGenStep,
  //pub keyset:PublicKeySet,
  //pub secret_share: Option<SecretKeyShare>,
}

impl KeyGenState
{
  pub fn clean_queues(&mut self)
  {
    if !self.expected_acks.is_empty()
    {
      let rf = &mut self.expected_acks[0];
      // let mut exp_src=&mut rf.0;
      let mut eacks = &mut rf.1;
      while eacks.is_empty()
      {
        self.expected_acks.pop_front();
        if self.expected_acks.is_empty()
        {
          self.is_done = true;
          return;
        }
        //exp_src=&mut self.expected_acks[0].0;
        eacks = &mut self.expected_acks[0].1;
      }
    }
  }
  pub fn is_next_sender(&self, sender: &NodeId) -> bool
  {
    if !self.expected_parts.is_empty()
    {
      return self.expected_parts[0] == *sender;
    }
    if !self.expected_acks.is_empty()
    {
      let (exp_src, _) = &self.expected_acks[0];
      return *exp_src == *sender;
    }
    return false;
  }
  pub fn is_next_message(&self, sender: &NodeId, msg: &SyncKeyGenMessage) -> bool
  {
    if !self.is_next_sender(sender)
    {
      return false;
    }
    match msg
    {
      SyncKeyGenMessage::Part(_) =>
      {
        if !self.expected_parts.is_empty()
        {
          return self.expected_parts[0] == *sender;
        }
        else
        {
          return false;
        }
      }
      SyncKeyGenMessage::Ack(src, _) =>
      {
        if !self.expected_parts.is_empty()
        {
          return false;
        }
        if !self.expected_acks.is_empty()
        {
          return self.expected_acks[0].0 == *sender &&
            !self.expected_acks[0].1.is_empty() &&
            self.expected_acks[0].1[0] == *src;
        }
        else
        {
          return false;
        }
      }
    }
  }
  pub fn process_part(&mut self, sender: &NodeId, part: Part) -> Vec<SyncKeyGenMessage>
  {
    let mut rng = rand::rngs::OsRng::new().expect("Could not open OS random number generator.");
    let outcome = match self.keygen.handle_part(sender, part, &mut rng)
    {
      Ok(outcome) => outcome,
      Err(e) =>
      {
        warn!("Failed processing part from {:?} {:?}", sender, e);
        return Vec::new();
      }
    };
    let mut ret: Vec<SyncKeyGenMessage> = Vec::new();
    match outcome
    {
      PartOutcome::Valid(Some(ack)) =>
      {
        self.expected_parts.pop_front();
        ret.push(SyncKeyGenMessage::Ack(sender.clone(), ack));
      }
      PartOutcome::Invalid(fault) =>
      {
        warn!("Faulty Part from {:?} :{:?}", sender, fault);
      }
      PartOutcome::Valid(None) =>
      {
        info!("We might be an observer");
        self.expected_parts.pop_front();
      }
    }
    ret
  }

  pub fn process_ack(&mut self, sender: &NodeId, _src: &NodeId, ack: Ack)
  {
    //let mut rng = rand::rngs::OsRng::new().expect("Could not open OS random number generator.");
    let ack_res = match self.keygen.handle_ack(sender, ack.clone())
    {
      Ok(outcome) => outcome,
      Err(e) =>
      {
        info!("Invalid Ack from {:?} : {:?}", sender, e);
        return;
      }
    };
    match ack_res
    {
      AckOutcome::Valid =>
      {}
      AckOutcome::Invalid(fault) =>
      {
        info!("Could not process Ack: {:?}", fault);
        return;
      }
    }
    self.expected_acks[0].1.pop_front();
    if self.expected_acks[0].1.is_empty()
    {
      self.expected_acks.pop_front();
      if self.expected_acks.is_empty()
      {
        self.is_done = true;
      }
    }
  }
  pub fn maybe_buffer(&mut self, sender: &NodeId, msg: SyncKeyGenMessage)
  {
    if let SyncKeyGenMessage::Part(_) = msg
    {
      if self.expected_parts.is_empty()
      {
        return;
      }
    }

    self.buffered_messages.push((sender.clone(), msg));
  }
  /// process incoming message and return generated acks, if any
  pub fn process_message(&mut self, sender: &NodeId, msg: SyncKeyGenMessage) -> Vec<SyncKeyGenMessage>
  {
    self.clean_queues();
    //check if we are done;
    let mut ret: Vec<SyncKeyGenMessage> = Vec::new();
    if self.expected_acks.is_empty() || self.is_done
    {
      self.is_done = true;
      return Vec::new();
    }
    if !self.is_next_message(sender, &msg)
    {
      self.maybe_buffer(sender, msg);
      return Vec::new();
    }
    let mut spl = sender.clone();
    let mut nmsg = msg;
    //let mut unproc = true;

loop
    {
      match nmsg
      {
        SyncKeyGenMessage::Part(parted) => ret.append(&mut self.process_part(&spl, parted)),
        SyncKeyGenMessage::Ack(src, ack) => self.process_ack(&spl, &src, ack),
      };
      let index = self
        .buffered_messages
        .iter()
        .position(|(snd, msg)| self.is_next_message(snd, msg));
      match index
      {
        Some(i) =>
        {
          let (aspl, anmsg) = self.buffered_messages.remove(i);
          spl = aspl;
          nmsg = anmsg;
          //unproc = true;
        }
        None =>
        {
         // unproc = false;
          break;
        }
      }
    }
    info!(
      "Processed keygen, still expecting {:?} Parts and {:?} base Acks",
      self.expected_parts.len(),
      self.expected_acks.len()
    );
    ret
  }
}

//#[derive(Encode,Decode)]
pub struct SharedConfig
{
  pub is_observer: bool,
  pub secret_share: Option<SecretKeyShare>,
  pub keyset: Option<PublicKeySet>,
  pub my_peer_id: PeerId,
  pub my_auth_id: AuthorityId,
  pub batch_size: u64,
}

pub enum BadgerState<B: BlockT, D>
where
  D: ConsensusProtocol<NodeId = NodeId>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
{
  /// Waiting for all initial validators to be connected
  AwaitingValidators,
  /// Running genesis keygen
  KeyGen(KeyGenState),
  /// Running completed Badger node
  Badger(BadgerNode<B, D>),

  /// need a Join Plan to become observer
  AwaitingJoinPlan,
}

pub struct JustificationCollector<B:BlockT>
{
 /// contemporary authorities for this block hash
 pub contemp_auth:Option<Vec<AuthorityId>>,
 pub justification:Vec<BadgerJustification<B>>
}
use network::ClientHandle as NetClient;
pub use sp_blockchain::{HeaderBackend,HeaderMetadata};
pub struct BadgerStateMachine<B: BlockT, D,Cl>
where
  D: ConsensusProtocol<NodeId = NodeId>, //specialize to avoid some of the confusion
  D::Message: Serialize + DeserializeOwned,
  B::Hash :Ord,
  Cl:NetClient<B>
{
  pub state: BadgerState<B, D>,
  pub peers: Peers,
  pub config: SharedConfig,
  pub queued_transactions: Vec<Vec<u8>>,
  pub persistent: BadgerPersistentData,
  pub keystore: KeyStorePtr,
  pub cached_origin: Option<AuthorityPair>,
  pub queued_messages: Vec<(u64, GossipMessage<B>)>,
  pub justification_collector: BTreeMap<B::Hash,JustificationCollector<B>>,
  pub delayed_justifications:Vec<(u64,BadgerJustification<B> )>,
  pub client: Arc<Cl>,
  pub finalizer: Box<dyn FnMut( &B::Hash,Option<Justification>)->bool+Send+Sync>,
}

const MAX_QUEUE_LEN: usize = 1024;

use threshold_crypto::serde_impl::SerdeSecret;

#[derive(Serialize, Deserialize, Debug)]
pub struct BadgerAuxCrypto
{
  pub secret_share: Option<SerdeSecret<SecretKeyShare>>,
  pub key_set: PublicKeySet,
  pub set_id: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SyncKeyGenMessage
{
  Part(Part),
  Ack(NodeId, Ack),
}

pub struct BadgerContainer<Block: BlockT, N: Network<Block>,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  pub val: Arc<BadgerGossipValidator<Block,Cl>>,
  pub network: N,
}
use crate::aux_store;

impl<B, N,Cl> BadgerHandler<B> for BadgerContainer<B, N,Cl>
where
  B: BlockT,
  N: Network<B>,
  Cl:NetClient<B>,
  B::Hash:Ord,
{
  fn get_current_authorities(&self)->AuthorityList
  {
    let cloned:Vec<_>;
    {
    let locked=self.val.inner.read();
    cloned=locked.persistent.authority_set.inner.read().current_authorities.iter().cloned().collect();
    }
    cloned
  }

  fn emit_justification(&self,hash:&B::Hash,auth_list:AuthorityList)
  {
  {
    let locked=self.val.inner.read();
    if !locked.is_authority() { return;}
  }
  self.val.do_emit_justification(hash, &self.network,auth_list)
  }
  fn vote_for_validators<SBe>(&self, auths: Vec<AuthorityId>, backend: &SBe) -> Result<(), Error>
  where
    SBe: AuxStore,
  {
    {
      let lock = self.val.inner.read();
      let aset = lock.persistent.authority_set.inner.read();
      let opt = aux_store::AuthoritySet {
        current_authorities: auths.clone(),
        self_id: aset.self_id.clone(),
        set_id: aset.set_id,
      };
      match aux_store::update_vote(
        &Some(opt),
        |insert| backend.insert_aux(insert, &[]),
        |delete| backend.insert_aux(&[], delete),
      )
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          warn!("Couldn't save vote to disk {:?}, might not be important", e);
        }
      }
    }

    self.val.do_vote_validators(auths, &self.network)
  }
  fn vote_change_encryption_schedule(&self, e: EncryptionSchedule) -> Result<(), Error>
  {
    self.val.do_vote_change_enc_schedule(e, &self.network)
  }
  fn notify_node_set(&self, _v: Vec<NodeId>) {}
  fn update_validators<SBe>(&self, new_validators: BTreeMap<NodeId, AuthorityId>, backend: &SBe)
  where
    SBe: AuxStore,
  {
    let nex_set;
    let aux: BadgerAuxCrypto;
    let self_id;
    {
      let mut lock = self.val.inner.write();
      lock.load_origin();
      {
        nex_set = lock.persistent.authority_set.inner.read().set_id + 1;
        self_id = lock.persistent.authority_set.inner.read().self_id.clone();
      }
      match lock.state
      {
        BadgerState::Badger(ref mut node) =>
        {
          let secr = node.algo.inner().netinfo().secret_key_share().clone();
          let ikset = node.algo.inner().netinfo().public_key_set();

          // let kset=Some(ikset.clone());
          aux = BadgerAuxCrypto {
            secret_share: match secr
            {
              Some(s) => Some(SerdeSecret(s.clone())),
              None => None,
            },
            key_set: ikset.clone(),
            set_id: (nex_set) as u32,
          };
        }
        _ =>
        {
          warn!("Not in BADGER state, odd.  Bailing");
          return;
        }
      };
    }
    {
      let lock = self.val.inner.write();
      lock
        .keystore
        .write()
        .insert_aux_by_type(app_crypto::key_types::HB_NODE, &self_id.encode(), &aux)
        .expect("Couldn't save keys");
    }

    {
      let mut lock = self.val.inner.write();

      lock.config.keyset = Some(aux.key_set);
      lock.config.secret_share = match aux.secret_share
      {
        Some(s) => Some(s.0.clone()),
        None => None,
      };

      let mut aset = lock.persistent.authority_set.inner.write();
      aset.current_authorities = new_validators.iter().map(|(_, v)| v.clone()).collect();
      aset.set_id = nex_set;
      match aux_store::update_authority_set(&aset, |insert| backend.insert_aux(insert, &[]))
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          warn!("Couldn't write to disk, potentially inconsistent state {:?}", e);
        }
      };

      let topic = badger_topic::<B>();
      let packet = {
        let ses_mes = SessionData {
          ses_id: aset.set_id,
          session_key: lock.cached_origin.as_ref().unwrap().public(),
          peer_id: lock.config.my_peer_id.clone().into(),
        };
        let sgn = lock.cached_origin.as_ref().unwrap().sign(&ses_mes.encode());
        SessionMessage { ses: ses_mes, sgn: sgn }
      };
      let packet_data = GossipMessage::<B>::Session(packet).encode();
      self.network.register_gossip_message(topic, packet_data);
    }
  }
}

pub struct WHash<A:AsRef<[u8]>>(A);
impl<A:AsRef<[u8]>> std::cmp::Ord  for WHash<A> 
{
  fn cmp(&self, other: &Self) -> std::cmp::Ordering 
  {
    let our_bytes=self.0.as_ref();
    let other_bytes=other.0.as_ref();
    if our_bytes.len()>other_bytes.len() {return  std::cmp::Ordering::Greater;}
    if our_bytes.len()<other_bytes.len() {return  std::cmp::Ordering::Less;}
    let miter=our_bytes.iter().zip(other_bytes.iter());
    for (a,b) in miter
    {
       let c=a.cmp(b);
       if c!= std::cmp::Ordering::Equal
       {
         return c;
       }
    }
    std::cmp::Ordering::Equal
  }
}
impl<A:AsRef<[u8]>> std::cmp::Eq  for WHash<A> 
{
}

impl<A:AsRef<[u8]>> From<A> for WHash<A>
{
  fn from(inp:A) -> Self
  {
  WHash{0:inp}
  }
}

impl<A:AsRef<[u8]>> std::cmp::PartialEq  for WHash<A> 
{
  fn eq(&self, other: &Self) -> bool 
  {
    let our_bytes=self.0.as_ref();
    let other_bytes=other.0.as_ref();
    if our_bytes.len()>other_bytes.len() {return false;}
    if our_bytes.len()<other_bytes.len() {return  false;}
    let miter=our_bytes.iter().zip(other_bytes.iter());
    for (a,b) in miter
    {
       let c=a.cmp(b);
       if c!= std::cmp::Ordering::Equal
       {
         return false;
       }
    }
    true
  }
}


impl<A:AsRef<[u8]>> std::cmp::PartialOrd  for WHash<A> 
{
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering>
  {
    let our_bytes=self.0.as_ref();
    let other_bytes=other.0.as_ref();
    if our_bytes.len()>other_bytes.len() {return  Some(std::cmp::Ordering::Greater);}
    if our_bytes.len()<other_bytes.len() {return  Some(std::cmp::Ordering::Less);}
    let miter=our_bytes.iter().zip(other_bytes.iter());
    for (a,b) in miter
    {
       let c=a.cmp(b);
       if c!= std::cmp::Ordering::Equal
       {
         return Some(c);
       }
    }
    Some(std::cmp::Ordering::Equal)
  }
}


impl<B: BlockT,Cl> BadgerStateMachine<B, QHB,Cl>
where
Cl:NetClient<B>,
B::Hash:Ord,
{
  pub fn new(
    keystore: KeyStorePtr, self_peer: PeerId, batch_size: u64, persist: BadgerPersistentData,
    client:Arc<Cl>,finalizer: Box<dyn FnMut( &B::Hash,Option<Justification>)->bool+Send+Sync>,
  ) -> BadgerStateMachine<B, QHB,Cl>
  {
    let ap: AuthorityPair;
    let is_ob: bool;

    {
      let aset = persist.authority_set.inner.read();
      ap = keystore
        .read()
        .key_pair_by_type::<AuthorityPair>(&aset.self_id.clone().into(), app_crypto::key_types::HB_NODE)
        .expect("Needs private key to work (bsm:new)");
      is_ob = !aset.current_authorities.contains(&aset.self_id);
      info!("SELFID: {:?} {:?}", &aset.self_id, &ap.public());
    }
    BadgerStateMachine {
      state: BadgerState::AwaitingValidators,
      peers: Peers::new(),
      config: SharedConfig {
        is_observer: is_ob,
        secret_share: None,
        keyset: None,
        my_peer_id: self_peer.clone(),
        batch_size: batch_size,
        my_auth_id: ap.public(),
        
      },
      queued_transactions: Vec::new(),
      keystore: keystore.clone(),
      cached_origin: Some(ap),
      persistent: persist,
      queued_messages: Vec::new(),
      justification_collector:BTreeMap::new(),
      delayed_justifications:Vec::new(),
      client:client.clone(),
      finalizer:finalizer
    }
  }

  #[inline]
  pub fn load_origin(&mut self)
  {
    if self.cached_origin.is_none()
    {
      let pair: AuthorityPair;
      {
        pair = self
          .keystore
          .read()
          .key_pair_by_type::<AuthorityPair>(&self.config.my_auth_id.clone().into(), app_crypto::key_types::HB_NODE)
          .expect("Needs private key to work");
      }
      self.cached_origin = Some(pair);
    }
  }
  pub fn process_justification(n_jst:BadgerJustification<B>, existing:&mut JustificationCollector<B>)
  {
    //self.load_origin();

    if existing.contemp_auth.is_none()
    {
      warn!("Should never be the case! - no authorities for block");
      return;
    }
    if existing.justification.iter().find(|x| x.validator==n_jst.validator).is_none()
     {
      if existing.contemp_auth.as_ref().unwrap().iter().find(|&x| *x==n_jst.validator).is_some()
      {
      existing.justification.push(n_jst);
      }
    }
  }
  pub fn check_justification_completion(&mut self,hkey: &B::Hash)->bool
  {
    let  existing= match self.justification_collector.get_mut(hkey)
    {
      Some(k) => k,
      None => {return false;}
    };

    let authn=existing.contemp_auth.as_ref().unwrap().len();
    let tolerated= authn - badger::util::max_faulty(authn);
    let collected=existing.justification.len();
    if collected>=tolerated
    {//justification complete
      let hash=existing.justification[0].hash.clone();
      let full = BadgerFullJustification::<B>
      {
        hash:hash.clone(),
        commits: existing.justification.drain(..).map(|x| BadgerAuthCommit{validator:x.validator,sgn:x.sgn}).collect()
      };

      (self.finalizer)(&hash,Some(full.encode()));

      for tpl in self.delayed_justifications.iter_mut()
      {
        (*tpl).0+=1;
      }
      self.delayed_justifications.retain(|x| x.0<MAX_DELAYED_JUSTIFICATIONS);
      
      true
    }
    else
    {
      false
    }
  }
  pub fn initiate_block_justification(&mut self,n_jst:BadgerJustification<B>,auth_list:AuthorityList)
  {
    self.load_origin();
    let hkey=n_jst.hash.clone();
    {
  let  existing=self.justification_collector.entry(hkey.clone()).or_insert(
      JustificationCollector {
      contemp_auth:Some(auth_list),
      justification:Vec::new(),
       });
       if existing.contemp_auth.is_none()
       {
         warn!("Should never be the case!");
         return;
       }
  let mut remaining:Vec<_>=Vec::new();     
  for jst in self.delayed_justifications.drain(..)
   {
   if jst.1.hash==n_jst.hash
    {
    //justification
    Self::process_justification(jst.1,existing)
    }
    else
    {
    remaining.push(jst);
    }
   }
   self.delayed_justifications=remaining;
   Self::process_justification(n_jst, existing);
  }
  if self.check_justification_completion(&hkey)
  {
    self.justification_collector.remove(&hkey);
  }
  }
  pub fn queue_transaction(&mut self, tx: Vec<u8>) -> Result<(), Error>
  {
    if self.queued_transactions.len() >= MAX_QUEUE_LEN
    {
      return Err(Error::Badger("Too many transactions queued".to_string()));
    }
    self.queued_transactions.push(tx);
    Ok(())
  }

  pub fn push_transaction(
    &mut self, tx: Vec<u8>,
  ) -> Result<CpStep<QHB>, badger::sender_queue::Error<badger::queueing_honey_badger::Error>>
  {
    match &mut self.state
    {
      BadgerState::AwaitingValidators | BadgerState::AwaitingJoinPlan | BadgerState::KeyGen(_) =>
      {
        match self.queue_transaction(tx)
        {
          Ok(_) => Ok(CpStep::<QHB>::default()),
          Err(_) => Err(badger::sender_queue::Error::Apply(
            badger::queueing_honey_badger::Error::HandleMessage(badger::dynamic_honey_badger::Error::UnknownSender),
          )),
        }
      }
      BadgerState::Badger(ref mut node) => node.push_transaction(tx),
    }
  }

  pub fn proceed_to_badger(&mut self) -> Vec<(LocalTarget, GossipMessage<B>)>
  {
    self.load_origin();

    let node: BadgerNode<B, QHB> = BadgerNode::<B, QHB>::new(
      self.config.batch_size as usize,
      self.config.secret_share.clone(),
      self.persistent.authority_set.inner.read().current_authorities.clone(),
      self.config.keyset.clone().unwrap(),
      self.cached_origin.as_ref().unwrap().public(),
      self.config.my_peer_id.clone(),
      self.keystore.clone(),
      &self.peers,
    );
    self.state = BadgerState::Badger(node);
    let bypass: Vec<_> = self.queued_transactions.drain(..).collect();
    for tx in bypass.into_iter()
    {
      match self.push_transaction(tx)
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          info!("Error pushing queued {:?}", e);
        }
      }
    }
    Vec::new()
  }

  pub fn proceed_to_keygen(&mut self) -> Vec<(LocalTarget, GossipMessage<B>)>
  {
    self.load_origin();

    //check if we already have the necessary keys
    let aset = self.persistent.authority_set.inner.read().clone();
    let mut should_badger = false;
    let should_regen = match self
      .keystore
      .read()
      .get_aux_by_type::<BadgerAuxCrypto>(app_crypto::key_types::HB_NODE, &aset.self_id.encode())
    {
      Ok(data) =>
      {
        if aset.set_id == data.set_id
        {
          self.config.keyset = Some(data.key_set);
          self.config.secret_share = match data.secret_share
          {
            Some(ss) => Some(ss.0),
            None => None,
          };
          should_badger = true;
          false
        }
        else
        {
          self
            .keystore
            .write()
            .delete_aux(app_crypto::key_types::HB_NODE, &aset.self_id.encode())
            .expect("Could not delete keystore");

          true
        }
      }
      Err(_) => true,
    };
    if should_badger
    {
      return self.proceed_to_badger();
    }
    info!("Keygen: should regen :{:?}", &should_regen);
    if should_regen
    {
      let mut rng = rand::rngs::OsRng::new().expect("Could not open OS random number generator.");
      let secr: SecretKey = bincode::deserialize(&self.cached_origin.as_ref().unwrap().to_raw_vec()).unwrap();
      info!("Our secret key : {:?} pub: {:?}", &secr, &secr.public_key());
      let thresh = badger::util::max_faulty(aset.current_authorities.len());
      let val_pub_keys: PubKeyMap<NodeId> = Arc::new(
        aset
          .current_authorities
          .iter()
          .map(|x| {
            (if *x == self.cached_origin.as_ref().unwrap().public()
            {
              (
                std::convert::Into::<PeerIdW>::into(self.config.my_peer_id.clone()),
                x.clone().into(),
              )
            }
            else
            {
              (
                std::convert::Into::<PeerIdW>::into(
                  self
                    .peers
                    .inverse
                    .get(x)
                    .expect("All validators should be mapped at this point")
                    .clone(),
                ),
                x.clone().into(),
              )
            })
          })
          .collect(),
      );
      info!("VAL_PUB {:?} {:?}", &val_pub_keys, &self.config.my_peer_id);
      let (skg, part) = SyncKeyGen::new(
        self.config.my_peer_id.clone().into(),
        secr,
        val_pub_keys.clone(),
        thresh,
        &mut rng,
      )
      .expect("Failed to create SyncKeyGen! ");
      let mut template_pre: Vec<NodeId> = val_pub_keys.iter().map(|(k, _)| k.clone()).collect();
      template_pre.sort();
      let template: VecDeque<_> = template_pre.into_iter().collect();
      let mut state = KeyGenState {
        is_observer: self.config.is_observer,
        keygen: skg,
        part: None,
        threshold: thresh as usize,
        buffered_messages: Vec::new(),
        expected_parts: template.clone(),
        expected_acks: template.iter().map(|x| (x.clone(), template.clone())).collect(),
        is_done: false,
      };
      let mut ret = vec![];
      if let Some(parted) = part
      {
        let pid = self.config.my_peer_id.clone().into();
        let mut acks = state.process_message(&pid, SyncKeyGenMessage::Part(parted.clone()));
        info!("Generated {:?} ACKS", acks.len());
        if acks.len() > 0
        {
          acks.append(&mut state.process_message(&pid, acks[0].clone()));
        }
        acks.push(SyncKeyGenMessage::Part(parted.clone()));
        ret = acks
          .into_iter()
          .map(|msg| {
            (
              LocalTarget::AllExcept(BTreeSet::new()),
              GossipMessage::KeygenData(BadgeredMessage::new(
                self.cached_origin.as_ref().unwrap().clone(),
                &bincode::serialize(&msg).expect("Serialize error in inital keygen processing"),
              )),
            )
          })
          .collect();
      }

      self.state = BadgerState::KeyGen(state);
      info!("Returning {:?} values", ret.len());
      ret
    }
    else
    {
      //no need to regenerate, use existing saved data...
      Vec::new()
    }
  }

  pub fn is_authority(&self) -> bool
  {
    let aset = self.persistent.authority_set.inner.read();
    aset.current_authorities.contains(&aset.self_id)
  }

  pub fn process_decoded_message(&mut self, message: &GossipMessage<B>) -> (SAction, Vec<(LocalTarget, GossipMessage<B>)>)
  {
    let cset_id;
    {
      cset_id = self.persistent.authority_set.inner.read().set_id;
    }
    match message
    {
      GossipMessage::Session(ses_msg) =>
      {
        info!(
          "Session received from {:?} of {:?}",
          &ses_msg.ses.peer_id, &ses_msg.ses.session_key
        );
        if ses_msg.ses.ses_id == cset_id
        //??? needs session versions
        {
          let mut ret_msgs: Vec<(LocalTarget, GossipMessage<B>)> = Vec::new();
          self
            .peers
            .update_id(&ses_msg.ses.peer_id.0, ses_msg.ses.session_key.clone());
          //debug!("Adding session key for {:?} :{:}")
          if let BadgerState::AwaitingValidators = self.state
          {
            let ln = self.persistent.authority_set.inner.read().current_authorities.len();
            let mut cur;
            {
              let iaset = self.persistent.authority_set.inner.read();
              cur = iaset
                .current_authorities
                .iter()
                .filter(|&n| self.peers.inverse.contains_key(n))
                .count();
              if self.is_authority() &&
                !self
                  .peers
                  .inverse
                  .contains_key(&self.cached_origin.as_ref().unwrap().public())
              {
                cur = cur + 1;
              }
            }
            info!(
              "Currently {:?} validators of {:?}, {:?} inverses",
              cur,
              ln,
              &self.peers.inverse.len()
            );
            if cur == ln
            {
              ret_msgs = self.proceed_to_keygen();
            }
          }
          //repropagate
          return (SAction::PropagateOnce, ret_msgs);
        }
        else if ses_msg.ses.ses_id < cset_id
        {
          //discard
          return (SAction::Discard, Vec::new());
        }
        else if ses_msg.ses.ses_id > cset_id + 1
        {
          //?? cache/propagate?
          return (SAction::Discard, Vec::new());
        }
        else
        {
          return (SAction::QueueRetry(ses_msg.ses.ses_id.into()), Vec::new());
        }
      }
      GossipMessage::KeygenData(wkgen) =>
      {
        info!("Got Keygen message!");

        /*
        Got data from key generation. If we are still waiting for peers, cache it for the moment.
        If we are done with keygen and are in badger state... hmm, this is gossip. Ignore and propagate?
        In the keygen state, try to consume it
        */
        let orid: PeerIdW = match self.peers.inverse.get(&wkgen.originator)
        {
          Some(dat) => dat.clone().into(),
          None =>
          {
            info!("Unknown originator for {:?}", &wkgen.originator);
            return (SAction::QueueRetry(0), Vec::new());
          }
        };
        info!("Originator: {:?}, Peer: {:?}", &wkgen.originator, &orid);
        if let BadgerState::KeyGen(ref mut step) = &mut self.state
        {
          let k_message: SyncKeyGenMessage = match bincode::deserialize(&wkgen.data)
          {
            Ok(data) => data,
            Err(_) =>
            {
              warn!("Keygen message should be correct");
              return (SAction::Discard, Vec::new());
            }
          };
          let acks = step.process_message(&orid, k_message);
          if step.is_done
          {
            info!("Initial keygen ready, generating... ");
            info!("Pub keys: {:?}", step.keygen.public_keys());
            let (kset, shr) = step.keygen.generate().expect("Initial key generation failed!");

            let aux = BadgerAuxCrypto {
              secret_share: match shr
              {
                Some(ref sec) =>
                {
                  info!("Gensec {:?}", sec.public_key_share());
                  Some(SerdeSecret(sec.clone()))
                }

                None => None,
              },
              key_set: kset.clone(),
              set_id: cset_id as u32,
            };

            self.config.keyset = Some(kset);
            self.config.secret_share = shr;
            {
              self
                .keystore
                .write()
                .insert_aux_by_type(
                  app_crypto::key_types::HB_NODE,
                  &self.persistent.authority_set.inner.read().self_id.encode(),
                  &aux,
                )
                .expect("Couldn't save keys");
            }

            (SAction::PropagateOnce, self.proceed_to_badger())
          }
          else
          {
            let ret = acks
              .into_iter()
              .map(|msg| {
                (
                  LocalTarget::AllExcept(BTreeSet::new()),
                  GossipMessage::KeygenData(BadgeredMessage::new(
                    self.cached_origin.as_ref().unwrap().clone(),
                    &bincode::serialize(&msg).expect("Serialize error in  keygen processing"),
                  )),
                )
              })
              .collect();
            (SAction::PropagateOnce, ret)
          }
        }
        else
        {
          info!("Keygen data received while out of keygen");
          if let BadgerState::AwaitingValidators = &mut self.state
          {
            // should buffer here?
            return (SAction::QueueRetry(1), Vec::new());
          }
          // propagate once?
          return (SAction::PropagateOnce, Vec::new());
        }
      }
      GossipMessage::BadgerData(bdat) =>
      {
        let orid: PeerIdW = match self.peers.inverse.get(&bdat.originator)
        {
          Some(dat) => dat.clone().into(),
          None =>
          {
            info!("Unknown originator for {:?}", &bdat.originator);
            return (SAction::QueueRetry(0), Vec::new());
          }
        };
        //we actually need to process observer state updates if we want to use SendQueue
        /*if !self
          .persistent
          .authority_set
          .inner
          .read()
          .current_authorities
          .contains(&bdat.originator)
        {
          info!("Got Badger message from non-authority, discarding");
          return (SAction::Discard, Vec::new());
        }*/
        match self.state
        {
          BadgerState::Badger(ref mut badger) =>
          {
            info!("BadGER: got gossip message uid: {}", &bdat.uid,);
            if let Ok(msg) = bincode::deserialize::<<QHB as ConsensusProtocol>::Message>(&bdat.data)
            {
              match badger.handle_message(&orid, msg)
              {
                Ok(_) =>
                {
                  //send is handled separately. trigger propose? or leave it for stream
                  debug!("BadGER: decoded gossip message");

                  return (SAction::Discard, Vec::new());
                }
                Err(e) =>
                {
                  info!("Error handling badger message {:?}", e);
                  telemetry!(CONSENSUS_DEBUG; "afg.err_handling_msg"; "err" => ?format!("{}", e));
                  return (SAction::Discard, Vec::new());
                }
              }
            }
            else
            {
              return (SAction::Discard, Vec::new());
            }
          }
          BadgerState::KeyGen(_) | BadgerState::AwaitingValidators =>
          {
            return (SAction::QueueRetry(2), Vec::new());
          }
          _ =>
          {
            warn!("Discarding badger message");
            return (SAction::Discard, Vec::new());
          }
        }
      },
      GossipMessage::JustificationData(just) =>
      {
        if !just.verify()
        {
          return (SAction::Discard, Vec::new()); 
        }
        let b_id=BlockId::Hash(just.hash);
        let hd=self.client.header(&b_id);
        let info = self.client.info().chain;
        let finalized_number = info.finalized_number;

        match hd 
        {
         Ok(opt) =>
         {
           if let Some(hdr)=opt
           {
           if *hdr.number()<=finalized_number
            {
              //we already have justification, hopefully
              info!("Discarding old justification");
              return  (SAction::Discard, Vec::new());
            }
           }
         }
         Err(_) =>{}
        }
        info!("Got justification for {:?}",&just.hash);
        let hkey=just.hash.clone();
         match self.justification_collector.get_mut(&hkey)
         {
           Some(existing) => {
            Self::process_justification(just.clone(),existing);
            if self.check_justification_completion(&hkey)
            {
              self.justification_collector.remove(&hkey);
            }

           },
           None =>
           {
             if self.delayed_justifications.iter().find( |x| (x.1.hash==just.hash && x.1.validator==just.validator)).is_some()
             {
                return  (SAction::Discard, Vec::new());
             }
          self.delayed_justifications.push((0,just.clone()));
           }
         }
          
         
        
     
       (SAction::PropagateOnce,Vec::new())
      }
    }
  }
  pub fn vote_change_encryption_schedule(&mut self, e: EncryptionSchedule) -> Result<(), &'static str>
  {
    match self.state
    {
      BadgerState::Badger(ref mut badger) =>
      {
        info!("BadGER: voting to change encrypt schedule ");

        match badger.vote_change_encryption_schedule(e)
        {
          Ok(_) =>
          {
            debug!("BadGER: voted");
            return Ok(());
          }
          Err(e) =>
          {
            info!("Error handling badger vote {:?}", e);
            return Err(e);
          }
        }
      }
      _ =>
      {
        warn!("Invalid state for voting");
        return Err("Invalid state".into());
      }
    }
  }
  pub fn vote_for_validators(&mut self, auths: Vec<AuthorityId>) -> Result<(), &'static str>
  {
    let mut map: BTreeMap<PeerIdW, PublicKey> = BTreeMap::new();
    for au in auths.into_iter()
    {
      if let Some(pid) = self.peers.inverse.get(&au)
      {
        map.insert(pid.clone().into(), au.into());
      }
      else
      {
        info!("No nodeId for {:?}", au);
        return Err("Missing NodeId".into());
      }
    }
    match self.state
    {
      BadgerState::Badger(ref mut badger) =>
      {
        info!("BadGER: voting to change authority set ");

        match badger.vote_for_validators(map)
        {
          Ok(_) =>
          {
            debug!("BadGER: voted");
            return Ok(());
          }
          Err(e) =>
          {
            info!("Error handling badger vote {:?}", e);
            return Err(e);
          }
        }
      }
      _ =>
      {
        warn!("Invalid state for voting");
        return Err("Invalid state".into());
      }
    }
  }

  pub fn process_message(
    &mut self, _who: &PeerId, mut data: &[u8],
  ) -> (SAction, Vec<(LocalTarget, GossipMessage<B>)>, Option<GossipMessage<B>>)
  {
    match GossipMessage::decode(&mut data)
    {
      Ok(message) =>
      {
        info!("GOt message from {:?} :{:?}", _who, &message);
        if !message.verify()
        {
          warn!("Invalid message signature in {:?}", &message);
          return (SAction::Discard, Vec::new(), None);
        }
        let (a, b) = self.process_decoded_message(&message);
        return (a, b, Some(message));
      }
      Err(e) =>
      {
        info!(target: "afg", "Error decoding message {:?}",e);
        telemetry!(CONSENSUS_DEBUG; "afg.err_decoding_msg"; "" => "");

        (SAction::Discard, Vec::new(), None)
      }
    }
  }
  pub fn process_and_replay(
    &mut self, who: &PeerId, data: &[u8],
  ) -> (Vec<(SAction, Option<GossipMessage<B>>)>, Vec<(LocalTarget, GossipMessage<B>)>)
  {
    let mut acc: Vec<(LocalTarget, GossipMessage<B>)> = Vec::new();
    let mut vals: Vec<(SAction, Option<GossipMessage<B>>)> = Vec::new();
    let (action, mut repl, msg) = self.process_message(who, data);
    acc.append(&mut repl);
    match action
    {
      SAction::QueueRetry(tp) =>
      {
        //message has not been processed
        match msg
        {
          Some(msg) => self.queued_messages.push((tp, msg)),
          None =>
          {}
        }
        return (vec![(SAction::Discard, None)], acc); //no need to inform upstream, I think
      }
      act =>
      {
        //retry existing messages...
        vals.push((act, msg));
        let mut done = false;
        let mut iter = 0;
        while !done
        {
          let mut new_queue = Vec::new();
          done = true;
          let silly_compiler: Vec<_> = self.queued_messages.drain(..).collect();
          info!("Replaying {:?}", silly_compiler.len());
          for (_, msg) in silly_compiler.into_iter()
          //TODO: use the number eventually
          {
            let (s_act, mut repl) = self.process_decoded_message(&msg);
            acc.append(&mut repl);
            match s_act
            {
              SAction::QueueRetry(tp) =>
              {
                new_queue.push((tp, msg));
              }
              SAction::Discard =>
              {
                done = false;
              }
              action =>
              {
                vals.push((action.clone(), Some(msg)));
                done = false;
              }
            }
          }
          self.queued_messages = new_queue;
          iter = iter + 1;
          if iter > 100
          //just in case, prevent deadlock
          {
            break;
          }
        }
        return (vals, acc);
      }
    }
  }
}

impl<B: BlockT> BadgerNode<B, QHB>
{
  fn push_transaction(
    &mut self, tx: Vec<u8>,
  ) -> Result<CpStep<QHB>, badger::sender_queue::Error<badger::queueing_honey_badger::Error>>
  {
    info!("BaDGER pushing transaction {:?}", &tx);
    let ret = self.algo.push_transaction(tx, &mut self.main_rng);
    info!("BaDGER pushed: complete ");
    ret
  }
}

impl<B: BlockT, D: ConsensusProtocol<NodeId = NodeId>> BadgerNode<B, D> where D::Message: Serialize + DeserializeOwned {}
use std::thread;
pub type BadgerNodeStepResult<D> = CpStep<D>;
pub type TransactionSet = Vec<Vec<u8>>; //agnostic?

impl<B: BlockT> BadgerNode<B, QHB>
//where
//  D::Message: Serialize + DeserializeOwned,
{
  pub fn new(
    batch_size: usize, sks: Option<SecretKeyShare>, validator_set: AuthorityList, pkset: PublicKeySet,
    auth_id: AuthorityId, self_id: PeerId, keystore: KeyStorePtr, peers: &Peers,
  ) -> BadgerNode<B, QHB>
  {
    let mut rng = OsRng::new().unwrap();

    //let ap:app_crypto::hbbft_thresh::Public=hex!["946252149ad70604cf41e4b30db13861c919d7ed4e8f9bd049958895c6151fab8a9b0b027ad3372befe22c222e9b733f"].into();
    let secr: SecretKey = match keystore
      .read()
      .key_pair_by_type::<AuthorityPair>(&auth_id, app_crypto::key_types::HB_NODE)
    {
      Ok(key) => bincode::deserialize(&key.to_raw_vec()).expect("Stored key should be correct"),
      Err(_) => panic!("SHould really have key at this point"),
    };
    let mut vset: Vec<NodeId> = validator_set
      .iter()
      .cloned()
      .map(|x| {
        if x == auth_id
        {
          self_id.clone().into()
        }
        else
        {
          Into::<NodeId>::into(peers.inverse.get(&x).expect("All auths should be known").clone())
        }
      })
      .collect();
    vset.sort();

    let ni = NetworkInfo::<NodeId>::new(self_id.clone().into(), sks, (pkset).clone(), vset);
    //let num_faulty = hbbft::util::max_faulty(indices.len());
    /*let peer_ids: Vec<_> = ni
    .all_ids()
    .filter(|&them| *them != PeerIdW { 0: self_id.clone() })
    .cloned()
    .collect();*/
    let val_map: BTreeMap<NodeId, PublicKey> = validator_set
      .iter()
      .map(|auth| {
        let nid = peers.inverse.get(auth).expect("All authorities should have loaded!");
        (nid.clone().into(), (*auth).clone().into())
      })
      .collect();

    let dhb = DynamicHoneyBadger::builder().build(ni, secr, Arc::new(val_map));
    let (qhb, qhb_step) = QueueingHoneyBadger::builder(dhb)
      .batch_size(batch_size)
      .build(&mut rng)
      .expect("instantiate QueueingHoneyBadger");

    let (sq, mut step) =
      SenderQueue::builder(qhb, peers.inner.keys().map(|x| (*x).clone().into())).build(self_id.clone().into());
    let output = step.extend_with(qhb_step, |fault| fault, BMessage::from);
    assert!(output.is_empty());
    let out_queue = step
      .messages
      .into_iter()
      .map(|msg| {
        let ser_msg = bincode::serialize(&msg.message).expect("serialize");
        SourcedMessage {
          sender_id: sq.our_id().clone(),
          target: msg.target.into(),
          message: ser_msg,
        }
      })
      .collect();
    let outputs = step.output.into_iter().collect();
    info!("BaDGER!! Initializing node");
    let node = BadgerNode {
      //id: self_id.clone(),
      node_id: self_id.clone().into(),
      algo: sq,
      main_rng: rng,
      // authorities: validator_set.clone(),
      //config: config.clone(),
      //in_queue: VecDeque::new(),
      out_queue: out_queue,
      outputs: outputs,
      _block: PhantomData,
    };
    node
  }
  fn process_step(
    &mut self, step: badger::sender_queue::Step<DynamicHoneyBadger<Vec<BadgerTransaction>, NodeId>>,
  ) -> Result<(), &'static str>
  {
    let out_msgs: Vec<_> = step
      .messages
      .into_iter()
      .map(|mmsg| {
        debug!("BaDGER!! Hundling  {:?} ", &mmsg.message);
        let ser_msg = bincode::serialize(&mmsg.message).expect("serialize");
        (mmsg.target, ser_msg)
      })
      .collect();
    self.outputs.extend(step.output.into_iter());
    debug!("BaDGER!! OK message, outputs: {} ", self.outputs.len());
    for (target, message) in out_msgs
    {
      self.out_queue.push_back(SourcedMessage {
        sender_id: self.node_id.clone(),
        target: target.into(),
        message,
      });
    }

    Ok(())
  }
  pub fn vote_change_encryption_schedule(&mut self, e: EncryptionSchedule) -> Result<(), &'static str>
  {
    match self.algo.vote_for(Change::EncryptionSchedule(e), &mut self.main_rng)
    {
      Ok(step) =>
      {
        return self.process_step(step);
      }
      Err(e) =>
      {
        info!("Error voting: {:?}", e);
        return Err("Error voting");
      }
    }
  }
  pub fn vote_for_validators(&mut self, new_vals: BTreeMap<PeerIdW, PublicKey>) -> Result<(), &'static str>
  {
    match self
      .algo
      .vote_for(Change::NodeChange(Arc::new(new_vals)), &mut self.main_rng)
    {
      Ok(step) =>
      {
        return self.process_step(step);
      }
      Err(e) =>
      {
        info!("Error voting: {:?}", e);
        return Err("Error voting");
      }
    }
  }
  pub fn handle_message(&mut self, who: &NodeId, msg: <QHB as ConsensusProtocol>::Message) -> Result<(), &'static str>
  {
    debug!("BaDGER!! Handling message from {:?} {:?}", who, &msg);
    match self.algo.handle_message(who, msg, &mut self.main_rng)
    {
      Ok(step) =>
      {
        return self.process_step(step);
      }
      Err(_) => return Err("Cannot handle message"),
    }
  }
}
pub struct BadgerGossipValidator<Block: BlockT,Cl>
where
Block::Hash:Ord,
Cl:NetClient<Block>
{
  // peers: RwLock<Arc<Peers>>,
  inner: RwLock<BadgerStateMachine<Block, QHB,Cl>>,
  pending_messages: RwLock<BTreeMap<PeerIdW, Vec<Vec<u8>>>>,
}
impl<Block: BlockT,Cl> BadgerGossipValidator<Block,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  fn send_message_either<N: Network<Block>>(
    &self, who: &PeerId, vdata: Vec<u8>, context_net: Option<&N>,
    context_val: &mut Option<&mut dyn ValidatorContext<Block>>,
  )
  {
    if let Some(context) = context_net
    {
      context.send_message(vec![who.clone()], vdata);
      return;
    }
    if let Some(context) = context_val
    {
      context.send_message(who, vdata);
    }
  }
  fn flush_message_either<N: Network<Block>>(
    &self, additional: &mut Vec<(LocalTarget, GossipMessage<Block>)>, context_net: Option<&N>,
    context_val: &mut Option<&mut dyn ValidatorContext<Block>>,
  )
  {
    info!("BaDGER!! Enter flush {:?}", thread::current().id());
    // let topic = badger_topic::<Block>();
    let sid: PeerId;
    let pair: AuthorityPair;
    let mut drain: Vec<_> = Vec::new();
    {
      info!("Lock inner");
      let mut locked = self.inner.write();
      sid = locked.config.my_peer_id.clone();
      pair = locked.cached_origin.clone().unwrap();
      if let BadgerState::Badger(ref mut state) = &mut locked.state
      {
        debug!("BaDGER!! Flushing {} messages_net", &state.out_queue.len());
        drain = state
          .out_queue
          .drain(..)
          .map(|x| {
            (
              x.target,
              GossipMessage::BadgerData(BadgeredMessage::new(pair.clone(), &x.message)),
            )
          })
          .collect();
      }
      info!("UnLock inner");
      
    }
    drain.append(additional);
    {
      let mut ldict = self.pending_messages.write();
      let plist;
      {
      let inner = self.inner.read();
       plist = inner.peers.connected_peer_list();
      }
      for (k, v) in ldict.iter_mut()
      {
        if v.len() == 0
        {
          continue;
        }
        if plist.contains(&k.0)
        {
          for msg in v.drain(..)
          {
            info!("BaDGER!! RESending to {:?}", &k);
            self.send_message_either(&k.0, msg, context_net, context_val);
          }
        }
      }
    }
    for (target, msg) in drain
    {
      //let vdata = GossipMessage::BadgerData(BadgeredMessage::new(pair,msg.message)).encode();
      let vdata = msg.encode();
      match target
      {
        LocalTarget::Nodes(node_set) =>
        {
          let av_list;
          {
          let inner = self.inner.read();
          trace!("Nodes lock success");
          let peers = &inner.peers;
          av_list = peers.connected_peer_list();
          }
          for to_id in node_set.iter()
          {
            debug!("BaDGER!! Id_net {:?}", &to_id);
            info!("Sending message node {:?} to {:?}", &msg, &to_id,);
            if av_list.contains(&to_id.0)
            {
              self.send_message_either(&to_id.0, vdata.clone(), context_net, context_val);
            }
            else
            {
              let mut ldict = self.pending_messages.write();
              let stat = ldict.entry(to_id.clone()).or_insert(Vec::new());
              stat.push(vdata.clone());
            }
          }
        }
        LocalTarget::AllExcept(exclude) =>
        {
          debug!("BaDGER!! AllExcept  {}", exclude.len());
          let clist;
          let mut vallist: Vec<_>;

          {
            let locked = self.inner.write();
          info!("Allex lock success");
          
          let peers = &locked.peers;
          vallist = locked
            .persistent
            .authority_set
            .inner
            .read()
            .current_authorities
            .iter()
            .map(|x| {
              if *x == locked.config.my_auth_id
              {
                locked.config.my_peer_id.clone()
              }
              else
              {
                peers.inverse.get(x).expect("All auths should be known").clone()
              }
            })
            .collect();
            clist=peers.connected_peer_list();
          }
          info!("Excluding {:?}", &exclude);
          for pid in clist
            .iter()
            .filter(|n| !exclude.contains(&PeerIdW { 0: (*n).clone() }))
          {
            let tmp = pid.clone();
            if tmp != sid
            {
              info!("Sending message {:?} to {:?}", &msg, &tmp,);
              self.send_message_either(pid, vdata.clone(), context_net, context_val);
              info!("Sent");
            }
            vallist.retain(|x| *x != tmp);
          }

          if vallist.len() > 0
          {
            let mut ldict = self.pending_messages.write();
            for val in vallist.into_iter()
            {
              let stat = ldict.entry(val.clone().into()).or_insert(Vec::new());
              stat.push(vdata.clone());
            }

            // context.register_gossip_message(topic,vdata.clone());
          }
        }
      }
    }
    info!("BaDGER!! Exit flush {:?}", thread::current().id());
  }
  /// Create a new gossip-validator.
  pub fn new(keystore: KeyStorePtr, self_peer: PeerId, batch_size: u64, persist: BadgerPersistentData, client:Arc<Cl>,finalizer: Box<dyn FnMut( &Block::Hash,Option<Justification>)->bool+Send+Sync>,) -> Self
  {
    Self {
      inner: RwLock::new(BadgerStateMachine::<Block, QHB,Cl>::new(
        keystore, self_peer, batch_size, persist,client,finalizer
      )),
      pending_messages: RwLock::new(BTreeMap::new()),
    }
  }
  /// collect outputs from
  pub fn pop_output(&self) -> Option<<QHB as ConsensusProtocol>::Output>
  {
    let mut locked = self.inner.write();
    match &mut locked.state
    {
      BadgerState::Badger(ref mut node) =>
      {
        info!("OUTPUTS: {:?}", node.outputs.len());
        node.outputs.pop_front()
      }
      _ => None,
    }
  }

  pub fn is_validator(&self) -> bool
  {
    let rd = self.inner.read();
    rd.is_authority()
  }

  pub fn push_transaction<N: Network<Block>>(&self, tx: Vec<u8>, net: &N) -> Result<(), Error>
  {
    let mut do_flush = false;
    {
      let mut locked = self.inner.write();
      if let BadgerState::Badger(ref mut node) = &mut locked.state
      {
        do_flush = match node.push_transaction(tx)
        {
          Ok(step) =>
          {
            match node.process_step(step)
            {
              Ok(_) =>
              {}
              Err(e) => return Err(Error::Badger(e.to_string())),
            };

            node.out_queue.len() > 0
          }
          Err(e) => return Err(Error::Badger(e.to_string())),
        }
      }
    }
    //send messages out
    if do_flush
    {
      self.flush_message_either(&mut Vec::new(), Some(net), &mut None);
    }

    Ok(())
  }
  pub fn do_emit_justification<N: Network<Block>>(&self,hash:&Block::Hash, net: &N,auth_list:AuthorityList)
  {
    if !self.is_validator()
    {
      return;
    }
    info!("Emitting Justification");
    
    let sgn;
    let authid;
    {
      let mut inner=self.inner.write();
      inner.load_origin();
      sgn = inner.cached_origin.as_ref().unwrap().sign(&hash.encode());
      authid=inner.cached_origin.as_ref().unwrap().public();
    }
  let jus= BadgerJustification::<Block>
   {
    hash:hash.clone(),
     validator:authid,
     sgn:sgn 
   };
   {
    let mut inner=self.inner.write();
    inner.initiate_block_justification(jus.clone(),auth_list);
   }
   {
    self.flush_message_either(&mut vec![ (LocalTarget::AllExcept(BTreeSet::new()), GossipMessage::JustificationData(jus) ) ], Some(net), &mut None);
   }
  }
  pub fn do_vote_validators<N: Network<Block>>(&self, auths: Vec<AuthorityId>, net: &N) -> Result<(), Error>
  {
    if !self.is_validator()
    {
      info!("Non-validator cannot vote");
      return Err(Error::Badger("Non-validator".to_string()));
    }
    let mut do_flush = false;
    {
      info!("DO_VOTE_PRELOCK");
      let mut locked = self.inner.write();
      info!("DO_VOTE_LOCK");
      match locked.vote_for_validators(auths)
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          return Err(Error::Badger(e.to_string()));
        }
      }
      if let BadgerState::Badger(ref mut node) = &mut locked.state
      {
        do_flush = node.out_queue.len() > 0;
      }
    }
    info!("DO_VOTE");
    //send messages out
    if do_flush
    {
      self.flush_message_either(&mut Vec::new(), Some(net), &mut None);
    }

    Ok(())
  }
  pub fn do_vote_change_enc_schedule<N: Network<Block>>(&self, e: EncryptionSchedule, net: &N) -> Result<(), Error>
  {
    if !self.is_validator()
    {
      info!("Non-validator cannot vote");
      return Err(Error::Badger("Non-validator".to_string()));
    }
    let mut do_flush = false;
    {
      let mut locked = self.inner.write();
      match locked.vote_change_encryption_schedule(e)
      {
        Ok(_) =>
        {}
        Err(e) =>
        {
          return Err(Error::Badger(e.to_string()));
        }
      }
      if let BadgerState::Badger(ref mut node) = &mut locked.state
      {
        do_flush = node.out_queue.len() > 0;
      }
    }
    //send messages out
    if do_flush
    {
      self.flush_message_either(&mut Vec::new(), Some(net), &mut None);
    }

    Ok(())
  }
}
use gossip::PeerConsensusState;

impl<Block: BlockT,Cl> network_gossip::Validator<Block> for BadgerGossipValidator<Block,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,

{
  fn new_peer(&self, context: &mut dyn ValidatorContext<Block>, who: &PeerId, _roles: Roles)
  {
    info!("New Peer called {:?}", who);
    {
      let mut inner = self.inner.write();
      inner.peers.update_peer_state(who, PeerConsensusState::Connected);
    };
    let packet = {
      let mut inner = self.inner.write();
      inner.peers.new_peer(who.clone());
      inner.load_origin();

      let ses_mes = SessionData {
        ///TODO: mitigate sending of invalid peerid
        ses_id: inner.persistent.authority_set.inner.read().set_id,
        session_key: inner.cached_origin.as_ref().unwrap().public(),
        peer_id: inner.config.my_peer_id.clone().into(),
      };
      let sgn = inner.cached_origin.as_ref().unwrap().sign(&ses_mes.encode());
      SessionMessage { ses: ses_mes, sgn: sgn }
    };

    //if let Some(packet) = packet {
    let packet_data = GossipMessage::<Block>::Session(packet).encode();
    context.send_message(who, packet_data);
    //}
  }

  fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId)
  {
    self.inner.write().peers.peer_disconnected(who);
  }

  fn validate(
    &self, context: &mut dyn ValidatorContext<Block>, who: &PeerId, data: &[u8],
  ) -> network_gossip::ValidationResult<Block::Hash>
  {
    info!("Enter validate {:?}", who);
    let topic = badger_topic::<Block>();
    let (actions, mut accumulated_output) = self.inner.write().process_and_replay(who, data);

    let mut ret = network_gossip::ValidationResult::ProcessAndDiscard(topic);
    if accumulated_output.len() == 0
    //whatever? does this make any sense?
    {
      ret = network_gossip::ValidationResult::Discard;
    }
    let mut keep: bool = false; //a hack. TODO?

    for (action, msg) in actions.into_iter()
    {
      match action
      {
        SAction::Discard =>
        {}
        SAction::PropagateOnce =>
        {
          if let Some(msg) = msg
          {
            context.broadcast_message(topic, msg.encode().to_vec(), false);
          }
        }

        SAction::RepropagateKeep =>
        {
          /*if let Some(msg)=msg
          {
           let cmsg = ConsensusMessage {
            engine_id: HBBFT_ENGINE_ID,
            data: msg.encode().to_vec(),
             };
           context.register_gossip_message(topic, cmsg);
           }*/
          //ret=network_gossip::ValidationResult::ProcessAndKeep;
          keep = true;
        }
        SAction::QueueRetry(num) =>
        {
          //shouldn't reach here normally
          info!("Unexpected SAction::QueueRetry {:?}", num);
        }
      }
    }
    info!("Sending topic");
    context.send_topic(who, topic, false);

    {
      info!("Preflush");
      self.flush_message_either::<ShutUp>(&mut accumulated_output, None, &mut Some(context));
      info!("Postflush");
    }
    info!("Flushed");
    if keep
    {
      ret = network_gossip::ValidationResult::ProcessAndKeep(topic);
    }
    return ret;
  }

  fn message_allowed<'b>(&'b self) -> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'b>
  {
   // info!("MessageAllowed 2");
    Box::new(move |who, intent, topic, mut data| {
      // if the topic is not something we're keeping at the moment,
      // do not send.
      if *topic != badger_topic::<Block>()
      //only one topic, i guess we may add epochs eventually
      {
        return false;
      }

      if let MessageIntent::PeriodicRebroadcast = intent
      {
        return true; //might be useful
      }

      {
       match self.inner.read().peers.inner.get(who)
      {
        None => return false,
        Some(x) => x,
      };
    }
      match GossipMessage::<Block>::decode(&mut data)
      {
        Err(_) => false,
        Ok(GossipMessage::BadgerData(_)) => return true,
        Ok(GossipMessage::KeygenData(_)) => return true,
        Ok(GossipMessage::Session(_)) => true,
        Ok(GossipMessage::JustificationData(_)) =>return true,
      }
    })
  }

  fn message_expired<'b>(&'b self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'b>
  {
    //let inner = self.inner.read();
    Box::new(move |topic, mut data| {
      //only one topic, i guess we may add epochs eventually
      if topic != badger_topic::<Block>()
      {
        return true;
      }

      match GossipMessage::<Block>::decode(&mut data)
      {
        Err(_) => true,
        Ok(GossipMessage::Session(ses_data)) =>
        {
          ses_data.ses.ses_id >= self.inner.read().persistent.authority_set.inner.read().set_id
        }
        Ok(_) => true,
      }
    })
  }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
struct ShutUp;

impl<B> Network<B> for ShutUp
where
  B: BlockT,
{
  type In = NetworkStream;
  fn local_id(&self) -> PeerId
  {
    PeerId::random()
  }

  fn messages_for(&self, _topic: B::Hash) -> Self::In
  {
    let (_tx, rx) = oneshot::channel::<mpsc::UnboundedReceiver<_>>();
    NetworkStream { outer: rx, inner: None }
  }

  fn register_validator(&self, _validator: Arc<dyn network_gossip::Validator<B>>) {}

  fn gossip_message(&self, _topic: B::Hash, _data: Vec<u8>, _force: bool) {}

  fn register_gossip_message(&self, _topic: B::Hash, _data: Vec<u8>) {}

  fn send_message(&self, _who: Vec<network::PeerId>, _data: Vec<u8>) {}

  fn report(&self, _who: network::PeerId, _cost_benefit: i32) {}

  fn announce(&self, _block: B::Hash, _data: Vec<u8>) {}
}

impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>>
where
  B: BlockT,
  S: network::specialization::NetworkSpecialization<B>,
  H: network::ExHashT,
{
  type In = NetworkStream;
  fn local_id(&self) -> PeerId
  {
    self.local_peer_id()
  }

  fn messages_for(&self, topic: B::Hash) -> Self::In
  {
    let (tx, rx) = oneshot::channel::<mpsc::UnboundedReceiver<_>>();
    self.with_gossip(move |gossip, _| {
      let inner_rx = gossip.messages_for(HBBFT_ENGINE_ID, topic);
      let _ = tx.send(inner_rx);
    });
    NetworkStream { outer: rx, inner: None }
  }

  fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<B>>)
  {
    self.with_gossip(move |gossip, context| gossip.register_validator(context, HBBFT_ENGINE_ID, validator))
  }

  fn gossip_message(&self, topic: B::Hash, data: Vec<u8>, force: bool)
  {
    let msg = ConsensusMessage {
      engine_id: HBBFT_ENGINE_ID,
      data,
    };

    self.with_gossip(move |gossip, ctx| gossip.multicast(ctx, topic, msg, force))
  }

  fn register_gossip_message(&self, topic: B::Hash, data: Vec<u8>)
  {
    let msg = ConsensusMessage {
      engine_id: HBBFT_ENGINE_ID,
      data,
    };

    self.with_gossip(move |gossip, _| gossip.register_message(topic, msg))
  }

  fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>)
  {
    let msg = ConsensusMessage {
      engine_id: HBBFT_ENGINE_ID,
      data,
    };

    self.with_gossip(move |gossip, ctx| {
      for who in &who
      {
        gossip.send_message(ctx, who, msg.clone())
      }
    })
  }

  fn report(&self, who: network::PeerId, cost_benefit: i32)
  {
    self.report_peer(who, ReputationChange::new( cost_benefit,"silly"));
  }

  fn announce(&self, block: B::Hash, associated_data: Vec<u8>)
  {
    info!("Announcing block!");
    self.announce_block(block, associated_data)
  }
}

/// A stream used by NetworkBridge in its implementation of Network.
pub struct NetworkStream
{
  inner: Option<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
  outer: oneshot::Receiver<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
}

impl Stream for NetworkStream
{
  type Item = network_gossip::TopicNotification;

  fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>>
  {
    if let Some(ref mut inner) = self.inner
    {
      match inner.try_next()
      {
        Ok(Some(opt)) => return Poll::Ready(Some(opt)),
        Ok(None) => return Poll::Pending,
        Err(_) => return Poll::Pending,
      };
    }
    match self.outer.try_recv()
    {
      Ok(Some(mut inner)) =>
      {
        let poll_result = match inner.try_next()
        {
          Ok(Some(opt)) => Poll::Ready(Some(opt)),
          Ok(None) => Poll::Pending,
          Err(_) => Poll::Pending,
        };
        self.inner = Some(inner);
        poll_result
      }
      Ok(None) => Poll::Pending,
      Err(_) => Poll::Pending,
    }
  }
}

/// Bridge between the underlying network service, gossiping consensus messages and Badger
pub struct NetworkBridge<B: BlockT, N: Network<B>,Cl>
where
Cl:NetClient<B>,
B::Hash:Ord,
{
  service: N,
  node: Arc<BadgerGossipValidator<B,Cl>>,
}

impl<B: BlockT, N: Network<B>,Cl> NetworkBridge<B, N,Cl>
where
Cl:NetClient<B>+'static,
B::Hash:Ord,
{
  /// Create a new NetworkBridge to the given NetworkService. Returns the service
  /// handle and a future that must be polled to completion to finish startup.
  /// If a voter set state is given it registers previous round votes with the
  /// gossip service.
  pub fn new(
    service: N, config: crate::Config, keystore: KeyStorePtr, persist: BadgerPersistentData,client:Arc<Cl>,finalizer: Box<dyn FnMut( &B::Hash,Option<Justification>)->bool+Send+Sync>,
  ) -> (Self, impl futures03::future::Future<Output = ()> + Send + Unpin)
  {
    let validator = BadgerGossipValidator::new(keystore, service.local_id().clone(), config.batch_size.into(), persist,client,finalizer);
    let validator_arc = Arc::new(validator);
    service.register_validator(validator_arc.clone());

    let bridge = NetworkBridge {
      service,
      node: validator_arc,
    };

    let startup_work = futures03::future::lazy(move |_| {
      // lazily spawn these jobs onto their own tasks. the lazy future has access
      // to tokio globals, which aren't available outside.
      //	let mut executor = tokio_executor::DefaultExecutor::current();
      //	executor.spawn(Box::new(reporting_job.select(on_exit.clone()).then(|_| Ok(()))))
      //		.expect("failed to spawn grandpa reporting job task");
      ()
    });

    (bridge, startup_work)
  }
  pub fn is_validator(&self) -> bool
  {
    self.node.is_validator()
  }

  /// Set up the global communication streams. blocks out transaction in. Maybe reverse of grandpa...
  pub fn global_communication(
    &self, is_voter: bool,
  ) -> (
    impl Stream<Item = <QHB as ConsensusProtocol>::Output>,
    impl SendOut,
    impl BadgerHandler<B>,
  )
  {
    let incoming = incoming_global::<B, N,Cl>(self.node.clone());

    let outgoing = TransactionFeed::<B, N,Cl>::new(self.service.clone(), is_voter, self.node.clone());
    let handler = BadgerContainer {
      val: self.node.clone(),
      network: self.service.clone(),
    };
    (incoming, outgoing, handler)
  }
}

fn incoming_global<B: BlockT, N: Network<B>,Cl>(
  gossip_validator: Arc<BadgerGossipValidator<B,Cl>>,
)-> impl Stream<Item = <QHB as ConsensusProtocol>::Output>
where
Cl:NetClient<B>,
B::Hash:Ord,
{
  BadgerStream::new(gossip_validator.clone())
}

impl<B: BlockT, N: Network<B>,Cl> Clone for NetworkBridge<B, N,Cl>
where
Cl:NetClient<B>,
B::Hash:Ord,
{
  fn clone(&self) -> Self
  {
    NetworkBridge {
      service: self.service.clone(),
      node: Arc::clone(&self.node),
    }
  }
}

pub trait BadgerHandler<B:BlockT>
{
  fn vote_for_validators<Be: AuxStore>(&self, auths: Vec<AuthorityId>, backend: &Be) -> Result<(), Error>;
  fn vote_change_encryption_schedule(&self, e: EncryptionSchedule) -> Result<(), Error>;
  fn notify_node_set(&self, vc: Vec<NodeId>); // probably not too important, though ve might want to make sure nodea are connected
 
  fn update_validators<Be: AuxStore>(&self, new_validators: BTreeMap<NodeId, AuthorityId>, backend: &Be);
  fn emit_justification(&self,hash:&B::Hash,auth_list:AuthorityList);
  fn get_current_authorities(&self) -> AuthorityList;
}

pub struct BadgerStream<Block: BlockT,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  validator: Arc<BadgerGossipValidator<Block,Cl>>,
}

impl<Block: BlockT,Cl> BadgerStream<Block,Cl>
where 
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  fn new(gossip_validator: Arc<BadgerGossipValidator<Block,Cl>>) -> Self
  {
    BadgerStream {
      validator: gossip_validator,
    }
  }
}

impl<Block: BlockT,Cl> Stream for BadgerStream<Block,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  type Item = <QHB as ConsensusProtocol>::Output;

  fn poll_next(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>>
  {
    match self.validator.pop_output()
    {
      Some(data) => Poll::Ready(Some(data)),
      None => Poll::Pending,
    }
  }
}

/// An output sink for commit messages.
struct TransactionFeed<Block: BlockT, N: Network<Block>,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  network: N,
  gossip_validator: Arc<BadgerGossipValidator<Block,Cl>>,
}

pub trait SendOut
{
  fn send_out(&mut self, input: Vec<Vec<u8>>) -> Result<(), Error>;
}

impl<Block: BlockT, N: Network<Block>,Cl> TransactionFeed<Block, N,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  pub fn new(network: N, _is_voter: bool, gossip_validator: Arc<BadgerGossipValidator<Block,Cl>>) -> Self
  {
    TransactionFeed {
      network,
      // is_voter,
      gossip_validator,
    }
  }
}
impl<Block: BlockT, N: Network<Block>,Cl> SendOut for TransactionFeed<Block, N,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  fn send_out(&mut self, input: Vec<Vec<u8>>) -> Result<(), Error>
  {
    for tx in input.into_iter().enumerate()
    {
      let locked = &self.gossip_validator;
      match locked.push_transaction(tx.1, &self.network)
      {
        Ok(_) =>
        {}
        Err(e) => return Err(e),
      }
    }
    Ok(())
  }
}

use network::consensus_gossip::ConsensusGossip;
use runtime_primitives::ConsensusEngineId;

struct NetworkSubtext<'g, 'p, B: BlockT>
{
  gossip: &'g mut ConsensusGossip<B>,
  protocol: &'p mut dyn network::Context<B>,
  engine_id: ConsensusEngineId,
}

impl<'g, 'p, B: BlockT> ValidatorContext<B> for NetworkSubtext<'g, 'p, B>
{
  fn broadcast_topic(&mut self, topic: B::Hash, force: bool)
  {
    self.gossip.broadcast_topic(self.protocol, topic, force);
  }

  fn broadcast_message(&mut self, topic: B::Hash, message: Vec<u8>, force: bool)
  {
    info!("mcast");
    self.gossip.multicast(
      self.protocol,
      topic,
      ConsensusMessage {
        data: message,
        engine_id: self.engine_id.clone(),
      },
      force,
    );
  }

  fn send_message(&mut self, who: &PeerId, message: Vec<u8>)
  {
    self.protocol.send_consensus(
      who.clone(),
      vec![ConsensusMessage {
        engine_id: self.engine_id,
        data: message,
      }],
    );
  }

  fn send_topic(&mut self, who: &PeerId, topic: B::Hash, force: bool)
  {
    self.gossip.send_topic(self.protocol, who, topic, self.engine_id, force);
  }
}

impl<Block: BlockT, N: Network<Block>,Cl> Sink<Vec<Vec<u8>>> for TransactionFeed<Block, N,Cl>
where
Cl:NetClient<Block>,
Block::Hash:Ord,
{
  type Error = Error;

  fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>>
  {
    Poll::Ready(Ok(()))
  }

  fn start_send(self: Pin<&mut Self>, input: Vec<Vec<u8>>) -> Result<(), Self::Error>
  {
    for tx in input.into_iter().enumerate()
    {
      let locked = &self.gossip_validator;
      match locked.push_transaction(tx.1, &self.network)
      {
        Ok(_) =>
        {}
        Err(e) => return Err(e),
      }
    }
    Ok(())
  }

  fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>>
  {
    Poll::Ready(Ok(()))
  }

  fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>>
  {
    Poll::Ready(Ok(()))
  }
}
