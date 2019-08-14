use codec::{Decode, Encode, Error as CodecError, Input};
use hbbft::{
	crypto::{
		Ciphertext, PublicKey as HBPublicKey, PublicKeySet, SecretKey as HBSecretKey, Signature,
	},
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
	Contribution, NodeIdT,
};
use hbbft_primitives::PublicKey;
use log::{debug, error, trace, warn};
use multihash::Multihash as PkHash;
use network::{config::Roles, PeerId};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::{min, Ordering};
use std::collections::{HashMap, VecDeque};
use std::convert::From;

#[derive(Clone, Hash, Debug, PartialEq, Eq)] // NodeIdT
pub struct NodeId(PkHash);

impl From<PeerId> for NodeId {
	fn from(pid: PeerId) -> Self {
		Self(PkHash::from_bytes(pid.as_bytes().to_vec()).unwrap())
	}
}

impl Ord for NodeId {
	fn cmp(&self, other: &Self) -> Ordering {
		self.0.as_bytes().cmp(other.0.as_bytes())
	}
}

impl PartialOrd for NodeId {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Serialize for NodeId {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_bytes(self.0.as_bytes())
	}
}

impl<'de> Deserialize<'de> for NodeId {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		match serde::Deserialize::deserialize(deserializer) {
			Ok(b) => Ok(Self(PkHash::from_bytes(b).unwrap())),
			Err(e) => Err(e),
		}
	}
}

impl Encode for NodeId {
	fn encode(&self) -> Vec<u8> {
		let bytes = self.clone().0.into_bytes();
		Encode::encode(&bytes)
	}
}

impl Decode for NodeId {
	fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError> {
		let bytes = Decode::decode(value)?;
		Ok(Self(PkHash::from_bytes(bytes).unwrap()))
	}
}

// unsafe impl Send for NodeId {}
// unsafe impl Sync for NodeId {}

#[derive(Clone, Debug)]
enum PeerPhase {
	Handshaking,
	PendingJoin { nid: NodeId, pk: PublicKey },
	EstablishedObserver { nid: NodeId, pk: PublicKey },
	EstablishedValidator { nid: NodeId, pk: PublicKey },
}

#[derive(Debug)]
struct PeerInfo {
	phase: PeerPhase,
}

impl Default for PeerInfo {
	fn default() -> Self {
		Self {
			phase: PeerPhase::Handshaking,
		}
	}
}

impl PeerInfo {
	fn new() -> Self {
		Self::default()
	}

	fn to_pending(&mut self, peer_info: (NodeId, PublicKey)) {
		self.phase = match self.phase {
			PeerPhase::Handshaking => PeerPhase::PendingJoin {
				nid: peer_info.0,
				pk: peer_info.1,
			},
			_ => panic!("Invalid state transition on `to_pending`"),
		}
	}

	fn to_observer(&mut self) {
		self.phase = match self.phase {
			PeerPhase::PendingJoin { ref nid, ref pk } => PeerPhase::EstablishedObserver {
				nid: nid.clone(),
				pk: pk.clone(),
			},
			_ => panic!("Invalid state transition on `to_observer`"),
		}
	}

	fn to_validator(&mut self, peer_info: Option<(NodeId, PublicKey)>) {
		self.phase = match self.phase {
			PeerPhase::Handshaking => match peer_info {
				Some((ref nid, ref pk)) => PeerPhase::EstablishedValidator {
					nid: nid.clone(),
					pk: pk.clone(),
				},
				None => panic!("Invalid state transition on `to_validator`"),
			},
			PeerPhase::EstablishedObserver { ref nid, ref pk } => PeerPhase::EstablishedValidator {
				nid: nid.clone(),
				pk: pk.clone(),
			},
			_ => panic!("Invalid state transition on `to_validator`"),
		}
	}

	pub fn node_id(&self) -> Option<&NodeId> {
		match &self.phase {
			PeerPhase::Handshaking => None,
			PeerPhase::PendingJoin { ref nid, .. } => Some(nid),
			PeerPhase::EstablishedObserver { ref nid, .. } => Some(nid),
			PeerPhase::EstablishedValidator { ref nid, .. } => Some(nid),
		}
	}

	pub fn public_key(&self) -> Option<&PublicKey> {
		match &self.phase {
			PeerPhase::Handshaking => None,
			PeerPhase::PendingJoin { ref pk, .. } => Some(pk),
			PeerPhase::EstablishedObserver { ref pk, .. } => Some(pk),
			PeerPhase::EstablishedValidator { ref pk, .. } => Some(pk),
		}
	}

	pub fn is_pending(&self) -> bool {
		match &self.phase {
			PeerPhase::PendingJoin { .. } => true,
			_ => false,
		}
	}
	pub fn is_observer(&self) -> bool {
		match &self.phase {
			PeerPhase::EstablishedObserver { .. } => true,
			_ => false,
		}
	}
	pub fn is_validator(&self) -> bool {
		match &self.phase {
			PeerPhase::EstablishedValidator { .. } => true,
			_ => false,
		}
	}
}

#[derive(Debug)]
pub(crate) struct Peers {
	map: HashMap<NodeId, PeerInfo>,
}

impl Default for Peers {
	fn default() -> Self {
		Peers {
			map: HashMap::new(),
		}
	}
}

impl Peers {
	pub fn add(&mut self, who: NodeId) {
		self.map.insert(who, PeerInfo::default());
	}

	pub fn del(&mut self, who: &NodeId) {
		self.map.remove(who);
	}

	pub fn set_validator(&mut self, who: &NodeId, pk: PublicKey) -> bool {
		// assert hash(pk) == who
		// return true if already validator
		let peer = self.map.get_mut(who).expect("Peer not found!");
		match peer.is_validator() {
			true => true,
			false => {
				peer.to_validator(Some((who.clone(), pk)));
				false
			}
		}
	}

	pub fn get_validator_key_map(&self) -> HashMap<NodeId, PublicKey> {
		let validators = self.map.values().filter(|p| p.is_validator());
		validators
			.map(|p| {
				(
					p.node_id().cloned().unwrap(),
					p.public_key().cloned().unwrap(),
				)
			})
			.collect()
	}
}
