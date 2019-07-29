use hbbft::{
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
	Contribution, NodeIdT,
};
use hbbft_primitives::PublicKey;
use log::{debug, error, trace, warn};
use multihash::Multihash as PkHash;
use network::config::Roles;
use parity_codec::{Decode, Encode, Input};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::{min, Ordering};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Hash, Debug, PartialEq, Eq)] // NodeIdT
pub(crate) struct PeerId(PkHash);

impl Ord for PeerId {
	fn cmp(&self, other: &Self) -> Ordering {
		self.0.as_bytes().cmp(other.0.as_bytes())
	}
}

impl PartialOrd for PeerId {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Serialize for PeerId {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.serialize_bytes(self.0.as_bytes())
	}
}

impl<'de> Deserialize<'de> for PeerId {
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

impl Encode for PeerId {
	fn encode(&self) -> Vec<u8> {
		let bytes = self.clone().0.into_bytes();
		Encode::encode(&bytes)
	}
}

impl Decode for PeerId {
	fn decode<I: Input>(value: &mut I) -> Option<Self> {
		let bytes = Decode::decode(value).unwrap();
		Some(Self(PkHash::from_bytes(bytes).unwrap()))
	}
}

unsafe impl Send for PeerId {}
unsafe impl Sync for PeerId {}

#[derive(Clone, Debug)]
enum PeerPhase {
	Handshaking,
	PendingJoin { pk: PublicKey },
	EstablishedObserver { pk: PublicKey },
	EstablishedValidator { pk: PublicKey },
}

struct PeerInfo {
	phase: PeerPhase,
}

impl PeerInfo {
	fn new(pk: Option<PublicKey>) -> Self {
		let phase = match pk {
			Some(pk) => PeerPhase::EstablishedValidator { pk },
			None => PeerPhase::Handshaking,
		};

		PeerInfo { phase }
	}

	fn to_pending(&mut self, pk: PublicKey) {
		self.phase = match self.phase {
			PeerPhase::Handshaking => PeerPhase::PendingJoin { pk },
			_ => panic!("Invalid state transition on `to_pending`"),
		}
	}

	fn to_observer(&mut self) {
		self.phase = match self.phase {
			PeerPhase::PendingJoin { ref pk } => PeerPhase::EstablishedObserver { pk: pk.clone() },
			_ => panic!("Invalid state transition on `to_observer`"),
		}
	}

	fn to_validator(&mut self, pk: Option<PublicKey>) {
		self.phase = match self.phase {
			PeerPhase::Handshaking => match pk {
				Some(ref pk) => PeerPhase::EstablishedValidator { pk: pk.clone() },
				None => panic!("Invalid state transition on `to_validator`"),
			},
			PeerPhase::EstablishedObserver { ref pk } => {
				PeerPhase::EstablishedValidator { pk: pk.clone() }
			}
			_ => panic!("Invalid state transition on `to_validator`"),
		}
	}

	pub fn public_key(&self) -> Option<&PublicKey> {
		match &self.phase {
			PeerPhase::Handshaking => None,
			PeerPhase::PendingJoin { ref pk } => Some(pk),
			PeerPhase::EstablishedObserver { ref pk } => Some(pk),
			PeerPhase::EstablishedValidator { ref pk } => Some(pk),
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

struct Peers {
	map: HashMap<PeerId, PeerInfo>,
}

impl Default for Peers {
	fn default() -> Self {
		Peers {
			map: HashMap::new(),
		}
	}
}

impl Peers {
	pub fn add(&mut self, who: PeerId, pk: Option<PublicKey>) {
		self.map.insert(who, PeerInfo::new(pk));
	}
	pub fn del(&mut self, who: &PeerId) {
		self.map.remove(who);
	}

	pub fn set_validator(&mut self, who: &PeerId, pk: PublicKey) -> bool {
		// assert hash(pk) == who
		let peer = self.map.get_mut(who).expect("Peer not found!");
		match peer.is_validator() {
			true => true,
			false => {
				peer.to_validator(Some(pk));
				false
			}
		}
	}
}
