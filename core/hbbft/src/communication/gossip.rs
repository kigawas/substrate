use hbbft_primitives::AuthorityId;
use network::config::Roles;
use network::consensus_gossip::{self as network_gossip, MessageIntent, ValidatorContext};
use parity_codec::{Decode, Encode};
use runtime_primitives::traits::{Block as BlockT, NumberFor, Zero};

use futures::prelude::*;
use futures::sync::mpsc;
use log::{debug, error, trace, warn};
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};

use hbbft::{
	crypto::{PublicKeySet, SecretKey, Signature},
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
	{Contribution, NodeIdT},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::peer::PeerId;
use hbbft_primitives::PublicKey;

#[derive(Debug)]
pub(super) enum KeyGenState {
	AwaitingPeers {
		required_peers: Vec<PeerId>,
		available_peers: Vec<PeerId>,
	},
	Generating {
		sync_key_gen: Option<SyncKeyGen<PeerId>>,
		public_key: Option<PublicKey>,
		public_keys: HashMap<PeerId, PublicKey>,

		part_count: u64,
		ack_count: u64,
	},
	Complete {
		sync_key_gen: Option<SyncKeyGen<PeerId>>,
		public_key: Option<PublicKey>,
	},
}

#[cfg_attr(feature = "derive-codec", derive(Encode, Decode))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyGenMessage {
	Part(Part),
	Ack(Ack),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkNodeInfo {
	pub(crate) nid: PeerId,
	pub(crate) pk: PublicKey,
}

type ActiveNetworkInfo = (
	Vec<NetworkNodeInfo>,
	PublicKeySet,
	HashMap<PeerId, PublicKey>,
);

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum NetworkState {
	Unknown(Vec<NetworkNodeInfo>),
	AwaitingPeers(Vec<NetworkNodeInfo>),
	GeneratingKeys(Vec<NetworkNodeInfo>, HashMap<PeerId, PublicKey>),
	Active(ActiveNetworkInfo),
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum GossipMessage {}
