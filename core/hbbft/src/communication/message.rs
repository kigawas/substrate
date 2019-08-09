use bincode;
use codec::{Decode, Encode, Error as CodecError, Input};
use hbbft::{
	crypto::{Ciphertext, PublicKey, PublicKeySet, SecretKey, Signature},
	dynamic_honey_badger::{
		Change as DhbChange, DynamicHoneyBadger, JoinPlan, KeyGenMessage as KG,
		Message as DhbMessage,
	},
	sync_key_gen::{Ack, Part, SyncKeyGen},
	Contribution as HbbftContribution, CpStep as MessagingStep, NodeIdT,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyGenMessage(KG);

impl Encode for KeyGenMessage {
	fn encode(&self) -> Vec<u8> {
		let encoded = bincode::serialize(&self).unwrap();
		let bytes = encoded.as_slice();
		Encode::encode(&bytes)
	}
}

impl Decode for KeyGenMessage {
	fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError> {
		let decoded: Vec<u8> = Decode::decode(value)?;
		let bytes = decoded.as_slice();
		Ok(bincode::deserialize(bytes).unwrap())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_message_encode_decode() {
		let kgm = KeyGenMessage(KG::Ack);
		println!("kgm {:?}", kgm);
	}
}
