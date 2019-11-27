#![cfg_attr(not(feature = "std"), no_std)]

mod mock;

#[cfg(test)]
mod tests;

use codec::{Codec, Decode, Encode};

use rstd::{fmt::Debug, marker::PhantomData, prelude::*};
use runtime_io::hashing::sha2_256;
use sr_primitives::{
	traits::{Hash, MaybeSerializeDeserialize, Member, SignedExtension, StaticLookup, Zero},
	transaction_validity::{
		InvalidTransaction, TransactionValidity, TransactionValidityError, ValidTransaction,
	},
	RuntimeDebug,
};

use generic_asset::{AssetOptions, Owner, PermissionLatest};
use support::dispatch::{Dispatchable, Result};
use support::traits::{Currency, Get, OnFreeBalanceZero, OnUnbalanced, Randomness, Time};
use support::{
	decl_event, decl_module, decl_storage, ensure, parameter_types, storage::child,
	weights::DispatchInfo, IsSubType, Parameter,
};
use system::{ensure_root, ensure_signed, RawOrigin};

pub trait Trait: system::Trait + generic_asset::Trait {
	type Time: Time;
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

// trait alias
type AssetModule<T> = generic_asset::Module<T>;
type AccountIdOf<T> = <T as system::Trait>::AccountId;
type AssetIdOf<T> = <T as generic_asset::Trait>::AssetId;
type BalanceOf<T> = <T as generic_asset::Trait>::Balance;
type MomentOf<T> = <<T as Trait>::Time as Time>::Moment;

// type alias
pub type SecretHash = [u8; 32]; // SHA-256 hash type
pub type Ticker = Vec<u8>;
pub type Secret = Vec<u8>;

#[derive(Encode, Decode, Default)]
pub struct TokenInfo<AssetId> {
	pub ticker: Ticker,
	pub asset_id: AssetId,
}

#[derive(Encode, Decode, Default)]
pub struct HtlcContractInfo<AccountId, Balance, Moment> {
	pub buyer: AccountId,
	pub ticker: Ticker,
	pub amount: Balance,
	pub expiration_in_ms: Moment,
	pub secret_hash: SecretHash,
}

decl_storage! {
	trait Store for Module<T: Trait> as Htlc {
		Token get(fn token_of):
			map Ticker => Option<TokenInfo<AssetIdOf<T>>>;
		Htlc get(fn htlc_of):
			map SecretHash => Option<HtlcContractInfo<AccountIdOf<T>, BalanceOf<T>, MomentOf<T>>>;
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {

		fn deposit_event() = default;

		pub fn create_token(origin, ticker: Ticker) -> Result {
			let who = ensure_signed(origin)?; // ensure root?

			// check if ticker created
			ensure!(!Self::token_exists(ticker.clone()), "ticker already exists");

			// build asset options
			let asset_options = Self::get_default_asset_options(who.clone());

			// create asset
			let next_asset_id = <AssetModule<T>>::next_asset_id();
			let _ =	<AssetModule<T>>::create_asset(Some(next_asset_id), Some(who), asset_options)?;

			let token_info = TokenInfo {
				ticker: ticker.clone(),
				asset_id: next_asset_id
			};
			<Token<T>>::insert(ticker, token_info);


			Ok(())
		}

		pub fn create_htlc(
			origin,
			ticker: Ticker,
			buyer: AccountIdOf<T>,
			amount: BalanceOf<T>,
			secret_hash: SecretHash,
			expiration_in_ms: MomentOf<T>
		) -> Result {
			let who = ensure_signed(origin)?; // root?

			let token = <Token<T>>::get(ticker);
			ensure!(token.is_some(), "ticker does not exist!");

			let asset_id = token.unwrap().asset_id;
			Self::mint(who, asset_id.clone(), buyer.clone(), amount);
			// <AssetModule<T>>::reserve(&asset_id, &buyer, amount);
			Ok(())
		}


		pub fn claim(origin, ticker: Ticker, secret: Secret) -> Result {
			// reserved -> free
			Ok(())
		}

		pub fn refund(origin, secret_hash: SecretHash) -> Result {
			// reserved -> burn
			let htlc =
			// Self::burn()
			Ok(())
		}

		pub fn do_something(origin, something: u32) -> Result {
			let who = ensure_signed(origin)?;


			// Something::put(something);

			// Self::deposit_event(RawEvent::SomethingStored(something, who));
			Ok(())
		}
	}
}

decl_event!(
	pub enum Event<T>
	where
		AccountId = AccountIdOf<T>,
		Amount = BalanceOf<T>,
	{
		// ticker, contract account id
		TokenCreated(Ticker, AccountId),
		// ticker, amount, buyer, secret_hash
		HtlcContractCreated(Ticker, Amount, AccountId, SecretHash),
	}
);

impl<T: Trait> Module<T> {
	fn hash_of(secret: Secret) -> SecretHash {
		sha2_256(&secret)
	}

	fn token_exists(ticker: Ticker) -> bool {
		<Token<T>>::exists(ticker)
	}

	fn htlc_exists(secret_hash: SecretHash) -> bool {
		<Htlc<T>>::exists(secret_hash)
	}

	fn get_default_asset_options(
		origin: AccountIdOf<T>,
	) -> AssetOptions<BalanceOf<T>, AccountIdOf<T>> {
		let default_permission = PermissionLatest {
			update: Owner::Address(origin.clone()),
			mint: Owner::Address(origin.clone()),
			burn: Owner::Address(origin),
		};
		AssetOptions {
			initial_issuance: <BalanceOf<T>>::zero(),
			permissions: default_permission,
		}
	}

	fn mint(asset_id: AssetIdOf<T>, who: AccountIdOf<T>, amount: BalanceOf<T>) -> Result {
		Ok(())
	}
}
