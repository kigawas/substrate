#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

use hbbft::{
	crypto::{PublicKey as HBPublicKey, SecretKey, Signature, PK_SIZE},
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
};

use client::decl_runtime_apis;
use codec::{Codec, Decode, Encode, Error as CodecError, Input};
use rstd::vec::Vec;
#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};
use sr_primitives::{
	traits::{DigestFor, NumberFor},
	ConsensusEngineId,
};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct PublicKey(HBPublicKey);

impl Default for PublicKey {
	fn default() -> Self {
		Self(HBPublicKey::from_bytes([0u8; PK_SIZE]).unwrap())
	}
}

#[cfg(feature = "std")]
impl Encode for PublicKey {
	fn encode(&self) -> Vec<u8> {
		Encode::encode(&self.0.to_bytes())
	}
}

#[cfg(feature = "std")]
impl Decode for PublicKey {
	fn decode<I: Input>(input: &mut I) -> Result<Self, CodecError> {
		let input_bytes = Decode::decode(input)?;
		match HBPublicKey::from_bytes(&input_bytes) {
			Ok(p) => Ok(PublicKey(p)),
			Err(_) => Err(CodecError::from("Decode public key error")),
		}
	}
}

#[derive(Debug, Default)] // we derive Default in order to use the clear() method in Drop
pub struct Keypair {
	/// The secret half of this keypair.
	pub secret: SecretKey,
	/// The public half of this keypair.
	pub public: PublicKey,
}

#[cfg(feature = "std")]
pub type AuthorityPair = Keypair;

/// Identity of a Grandpa authority.
pub type AuthorityId = PublicKey;

/// Signature for a Grandpa authority.
pub type AuthoritySignature = Signature;

/// The `ConsensusEngineId` of GRANDPA.
pub const HBBFT_ENGINE_ID: ConsensusEngineId = *b"HNBG";

/// The weight of an authority.
pub type AuthorityWeight = u64;

/// The index of an authority.
pub type AuthorityIndex = u64;

/// A scheduled change of authority set.
#[cfg_attr(feature = "std", derive(Debug, Serialize))]
#[derive(Clone, Eq, PartialEq, Encode, Decode)]
pub struct ScheduledChange<N> {
	/// The new authorities after the change, along with their respective weights.
	pub next_authorities: Vec<(AuthorityId, AuthorityWeight)>,
	/// The number of blocks to delay.
	pub delay: N,
}

#[cfg_attr(feature = "std", derive(Serialize, Debug))]
#[derive(Decode, Encode, PartialEq, Eq, Clone)]
pub enum ConsensusLog<N: Codec> {
	/// Schedule an authority set change.
	///
	/// Precedence towards earlier or later digest items can be given
	/// based on the rules of the chain.
	///
	/// No change should be scheduled if one is already and the delay has not
	/// passed completely.
	///
	/// This should be a pure function: i.e. as long as the runtime can interpret
	/// the digest type it should return the same result regardless of the current
	/// state.
	#[codec(index = "1")]
	ScheduledChange(ScheduledChange<N>),
	/// Force an authority set change.
	///
	/// Forced changes are applied after a delay of _imported_ blocks,
	/// while pending changes are applied after a delay of _finalized_ blocks.
	///
	/// Precedence towards earlier or later digest items can be given
	/// based on the rules of the chain.
	///
	/// No change should be scheduled if one is already and the delay has not
	/// passed completely.
	///
	/// This should be a pure function: i.e. as long as the runtime can interpret
	/// the digest type it should return the same result regardless of the current
	/// state.
	#[codec(index = "2")]
	ForcedChange(N, ScheduledChange<N>),
	/// Note that the authority with given index is disabled until the next change.
	#[codec(index = "3")]
	OnDisabled(AuthorityIndex),
	/// A signal to pause the current authority set after the given delay.
	/// After finalizing the block at _delay_ the authorities should stop voting.
	#[codec(index = "4")]
	Pause(N),
	/// A signal to resume the current authority set after the given delay.
	/// After authoring the block at _delay_ the authorities should resume voting.
	#[codec(index = "5")]
	Resume(N),
}

impl<N: Codec> ConsensusLog<N> {
	/// Try to cast the log entry as a contained signal.
	pub fn try_into_change(self) -> Option<ScheduledChange<N>> {
		match self {
			ConsensusLog::ScheduledChange(change) => Some(change),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained forced signal.
	pub fn try_into_forced_change(self) -> Option<(N, ScheduledChange<N>)> {
		match self {
			ConsensusLog::ForcedChange(median, change) => Some((median, change)),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained pause signal.
	pub fn try_into_pause(self) -> Option<N> {
		match self {
			ConsensusLog::Pause(delay) => Some(delay),
			_ => None,
		}
	}

	/// Try to cast the log entry as a contained resume signal.
	pub fn try_into_resume(self) -> Option<N> {
		match self {
			ConsensusLog::Resume(delay) => Some(delay),
			_ => None,
		}
	}
}

pub const PENDING_CHANGE_CALL: &str = "hbbft_pending_change";
pub const AUTHORITIES_CALL: &str = "hbbft_authorities";
pub const GET_THRESHOLD_SIGNATURE_CALL: &str = "get_threshold_signature";

decl_runtime_apis! {
	/// APIs for integrating the GRANDPA finality gadget into runtimes.
	/// This should be implemented on the runtime side.
	///
	/// This is primarily used for negotiating authority-set changes for the
	/// gadget. GRANDPA uses a signaling model of changing authority sets:
	/// changes should be signaled with a delay of N blocks, and then automatically
	/// applied in the runtime after those N blocks have passed.
	///
	/// The consensus protocol will coordinate the handoff externally.
	#[api_version(2)]
	pub trait HbbftApi {
		fn hbbft_pending_change(digest: &DigestFor<Block>)
			-> Option<ScheduledChange<NumberFor<Block>>>;


		fn hbbft_forced_change(digest: &DigestFor<Block>)
			-> Option<(NumberFor<Block>, ScheduledChange<NumberFor<Block>>)>;


		fn hbbft_authorities() -> Vec<(AuthorityId, AuthorityWeight)>;

		fn get_threshold_signature();
	}
}
