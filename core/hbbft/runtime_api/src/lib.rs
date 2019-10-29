#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;
use client::decl_runtime_apis;
pub mod app {
	use sr_primitives::app_crypto::{app_crypto, key_types::HB_NODE, hbbft_thresh};
	app_crypto!(hbbft_thresh, HB_NODE);
}


decl_runtime_apis! {

	
	pub trait HbbftApi {
		fn do_nothing();
	}
}
