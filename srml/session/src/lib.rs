// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! # Session Manager Module
//!
//! ## Overview
//! <!-- Original author of paragraph: @gavofyork -->
//!
//! The Session manager is provided with the validator set and allows them to manage their session keys for the consensus module.
//!
//! To use it in your runtime, you need to implement the session [`Trait`](./trait.Trait.html).
//!
//! The supported dispatchable functions are documented in the [`Call`](./enum.Call.html) enum.
//!
//! ### Terminology
//! <!-- Original author of paragraph: @gavofyork -->
//!
//! * **Set the Session Key of a Validator** A Session Key is set for a Validator using `set_key` so that they may use it in their next Session. It uses `NextKeyFor` to store a mapping between their `AccountID` and the Session Key that each of them provide. `set_key` is a public call (since it uses `ensure_signed` that checks that the origin is a signed account) since we want to allow users to set their Session Key prior to becoming a Validator, hence the Account ID of the origin that we are storing in `NextKeyFor` may not necessarily be associated with a block author or a validator. The Session module ensures that the Session Key of reaped (deleted) accounts are removed by removing the key once the account balance is zero.
//! * **Set the Session Keys of a Validator Set** Each Session we iterate through the current set of Validator Account IDs by index and check if a Session Key was created for it with `set_key` in the previous session. If it was then we call `set_authority` of the Consensus SRML and pass it a set of Session Keys (which it generates from the provided list of Account IDs corresponding to each of the new validators that are to be set) to set the Session Keys for the new Validator Set. We check if the Session Key of the current authority index matches the Session Key (if any) stored under their Validator Index in the `AuthorityStorageVec` mapping. Otherwise we update the mapping with their Session Key and update the saved list of original authorities if necessary (see https://github.com/paritytech/substrate/issues/1290). Note: Authorities are stored in the Consensus SRML. They are represented by an Index of the Validator Account ID from the Session SRML and allocated with a Session Key for the length of the Session.
//! * **Session Length** The Session Length is measured in block numbers and is set with `set_length` during a session. It is used in subsequent sessions. At the start of the next session we allocated a Session Index and record the timestamp when the session started. If a next Session Length was recorded in the previous session we record it as the new Session Length, and if its length does not correspond to the last block of the next session then we record it as the Last Length Change for the New Session.
//! * **Session Rotation** The Session is changed at the end of the final block of the current Session Length using the `on_finalise` method. It may be called by either an origin or internally from another SRML at the end of each block. The Session Rotation may be configured to be a Normal Session Rotation or an Exceptional Session Rotation.
//! * **Normal Session Rotation** A rewardable session wherer rewards are applied.
//! * **Exceptional Session Rotation** A slashable session.
//!
//! ### Goals
//!
//! The session system in Substrate is designed to make the following possible:
//!
//! * Set the Session Keys of a Validator Set for the next Session.
//! * Configure and switch between either Normal or Exceptional Sessions Rotations.
//!
//! ## Interface
//!
//! ### Dispatchable Functions
//!
//! * `set_key` - TODO.
//! * `set_length` - TODO.
//! * `force_new_session` - TODO.
//! * `on_finalize` - TODO.
//!
//! Please refer to the [`Call`](./enum.Call.html) enum and its associated variants for documentation on each function.
//!
//! ### Public Functions
//!
//! * `validator_count` - TODO.
//! * `last_length_change` - TODO.
//! * `apply_force_new_session` - TODO.
//! * `set_validators` - TODO.
//! * `check_rotate_session` - TODO.
//! * `rotate_session` - TODO.
//! * `ideal_session_duration` - TODO.
//! * `blocks_remaining` - TODO.
//!
//! Please refer to the [`Module`](https://crates.parity.io/srml_session/struct.Module.html) struct for details on publicly available functions.
//!
//! **Note:** When using the publicly exposed functions, you (the runtime developer) are responsible for implementing any necessary checks (e.g. that the sender is the signer) before calling a function that will affect storage.
//!
//! ## Usage
//!
//! ### Prerequisites
//!
//! Import the Session module and types and derive your runtime's configuration traits from the Session module trait.
//!
//! ### Simple Code Snippet
//!
//! See the tests contained in files in this module's directory for simple code snippets that may make this module's functionalities clearer.
//!
//! ## Related Modules
//!
//! * [`System`](../srml_system/index.html)
//! * [`Support`](../srml_support/index.html)

#![cfg_attr(not(feature = "std"), no_std)]

use rstd::prelude::*;
use primitives::traits::{As, Zero, One, Convert};
use srml_support::{StorageValue, StorageMap, for_each_tuple, decl_module, decl_event, decl_storage};
use srml_support::{dispatch::Result, traits::OnFreeBalanceZero};
use system::ensure_signed;
use rstd::ops::Mul;

/// A session has changed.
pub trait OnSessionChange<T> {
	/// Session has changed.
	fn on_session_change(time_elapsed: T, should_reward: bool);
}

macro_rules! impl_session_change {
	() => (
		impl<T> OnSessionChange<T> for () {
			fn on_session_change(_: T, _: bool) {}
		}
	);

	( $($t:ident)* ) => {
		impl<T: Clone, $($t: OnSessionChange<T>),*> OnSessionChange<T> for ($($t,)*) {
			fn on_session_change(time_elapsed: T, should_reward: bool) {
				$($t::on_session_change(time_elapsed.clone(), should_reward);)*
			}
		}
	}
}

for_each_tuple!(impl_session_change);

pub trait Trait: timestamp::Trait + consensus::Trait {
	type ConvertAccountIdToSessionKey: Convert<Self::AccountId, Option<Self::SessionKey>>;
	type OnSessionChange: OnSessionChange<Self::Moment>;
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event<T>() = default;

		/// Sets the session key of `_validator` to `_key`. This doesn't take effect until the next
		/// session.
		fn set_key(origin, key: T::SessionKey) {
			let who = ensure_signed(origin)?;
			// set new value for next session
			<NextKeyFor<T>>::insert(who, key);
		}

		/// Set a new session length. Won't kick in until the next session change (at current length).
		fn set_length(#[compact] new: T::BlockNumber) {
			<NextSessionLength<T>>::put(new);
		}

		/// Forces a new session.
		fn force_new_session(apply_rewards: bool) -> Result {
			Self::apply_force_new_session(apply_rewards)
		}

		fn on_finalize(n: T::BlockNumber) {
			Self::check_rotate_session(n);
		}
	}
}

decl_event!(
	pub enum Event<T> where <T as system::Trait>::BlockNumber {
		/// New session has happened. Note that the argument is the session index, not the block
		/// number as the type might suggest.
		NewSession(BlockNumber),
	}
);

decl_storage! {
	trait Store for Module<T: Trait> as Session {
		/// The current set of validators.
		pub Validators get(validators) config(): Vec<T::AccountId>;
		/// Current length of the session.
		pub SessionLength get(length) config(session_length): T::BlockNumber = T::BlockNumber::sa(1000);
		/// Current index of the session.
		pub CurrentIndex get(current_index) build(|_| T::BlockNumber::sa(0)): T::BlockNumber;
		/// Timestamp when current session started.
		pub CurrentStart get(current_start) build(|_| T::Moment::zero()): T::Moment;

		/// New session is being forced if this entry exists; in which case, the boolean value is whether
		/// the new session should be considered a normal rotation (rewardable) or exceptional (slashable).
		pub ForcingNewSession get(forcing_new_session): Option<bool>;
		/// Block at which the session length last changed.
		LastLengthChange: Option<T::BlockNumber>;
		/// The next key for a given validator.
		NextKeyFor build(|config: &GenesisConfig<T>| {
			config.keys.clone()
		}): map T::AccountId => Option<T::SessionKey>;
		/// The next session length.
		NextSessionLength: Option<T::BlockNumber>;
	}
	add_extra_genesis {
		config(keys): Vec<(T::AccountId, T::SessionKey)>;
	}
}

impl<T: Trait> Module<T> {
	/// The current number of validators.
	pub fn validator_count() -> u32 {
		<Validators<T>>::get().len() as u32
	}

	/// The last length change if there was one, zero if not.
	pub fn last_length_change() -> T::BlockNumber {
		<LastLengthChange<T>>::get().unwrap_or_else(T::BlockNumber::zero)
	}

	// INTERNAL API (available to other runtime modules)
	/// Forces a new session, no origin.
	pub fn apply_force_new_session(apply_rewards: bool) -> Result {
		<ForcingNewSession<T>>::put(apply_rewards);
		Ok(())
	}

	/// Set the current set of validators.
	///
	/// Called by `staking::new_era` only. `rotate_session` must be called after this in order to
	/// update the session keys to the next validator set.
	pub fn set_validators(new: &[T::AccountId]) {
		<Validators<T>>::put(&new.to_vec());
	}

	/// Hook to be called after transaction processing.
	pub fn check_rotate_session(block_number: T::BlockNumber) {
		// Do this last, after the staking system has had the chance to switch out the authorities for the
		// new set.
		// Check block number and call `rotate_session` if necessary.
		let is_final_block = ((block_number - Self::last_length_change()) % Self::length()).is_zero();
		let (should_end_session, apply_rewards) = <ForcingNewSession<T>>::take()
			.map_or((is_final_block, is_final_block), |apply_rewards| (true, apply_rewards));
		if should_end_session {
			Self::rotate_session(is_final_block, apply_rewards);
		}
	}

	/// Move on to next session: register the new authority set.
	pub fn rotate_session(is_final_block: bool, apply_rewards: bool) {
		let now = <timestamp::Module<T>>::get();
		let time_elapsed = now.clone() - Self::current_start();
		let session_index = <CurrentIndex<T>>::get() + One::one();

		Self::deposit_event(RawEvent::NewSession(session_index));

		// Increment current session index.
		<CurrentIndex<T>>::put(session_index);
		<CurrentStart<T>>::put(now);

		// Enact session length change.
		let len_changed = if let Some(next_len) = <NextSessionLength<T>>::take() {
			<SessionLength<T>>::put(next_len);
			true
		} else {
			false
		};
		if len_changed || !is_final_block {
			let block_number = <system::Module<T>>::block_number();
			<LastLengthChange<T>>::put(block_number);
		}

		T::OnSessionChange::on_session_change(time_elapsed, apply_rewards);

		// Update any changes in session keys.
		let v = Self::validators();
		<consensus::Module<T>>::set_authority_count(v.len() as u32);
		for (i, v) in v.into_iter().enumerate() {
			<consensus::Module<T>>::set_authority(
				i as u32,
				&<NextKeyFor<T>>::get(&v)
					.or_else(|| T::ConvertAccountIdToSessionKey::convert(v))
					.unwrap_or_default()
			);
		};
	}

	/// Get the time that should have elapsed over a session if everything was working perfectly.
	pub fn ideal_session_duration() -> T::Moment {
		let block_period: T::Moment = <timestamp::Module<T>>::minimum_period();
		let session_length: T::BlockNumber = Self::length();
		Mul::<T::BlockNumber>::mul(block_period, session_length)
	}

	/// Number of blocks remaining in this session, not counting this one. If the session is
	/// due to rotate at the end of this block, then it will return 0. If the session just began, then
	/// it will return `Self::length() - 1`.
	pub fn blocks_remaining() -> T::BlockNumber {
		let length = Self::length();
		let length_minus_1 = length - One::one();
		let block_number = <system::Module<T>>::block_number();
		length_minus_1 - (block_number - Self::last_length_change() + length_minus_1) % length
	}
}

impl<T: Trait> OnFreeBalanceZero<T::AccountId> for Module<T> {
	fn on_free_balance_zero(who: &T::AccountId) {
		<NextKeyFor<T>>::remove(who);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::RefCell;
	use srml_support::{impl_outer_origin, assert_ok};
	use runtime_io::with_externalities;
	use substrate_primitives::{H256, Blake2Hasher};
	use primitives::BuildStorage;
	use primitives::traits::{BlakeTwo256, IdentityLookup};
	use primitives::testing::{Digest, DigestItem, Header, UintAuthorityId, ConvertUintAuthorityId};

	impl_outer_origin!{
		pub enum Origin for Test {}
	}

	thread_local!{
		static NEXT_VALIDATORS: RefCell<Vec<u64>> = RefCell::new(vec![1, 2, 3]);
	}

	pub struct TestOnSessionChange;
	impl OnSessionChange<u64> for TestOnSessionChange {
		fn on_session_change(_elapsed: u64, _should_reward: bool) {
			NEXT_VALIDATORS.with(|v| Session::set_validators(&*v.borrow()));
		}
	}

	#[derive(Clone, Eq, PartialEq)]
	pub struct Test;
	impl consensus::Trait for Test {
		type Log = DigestItem;
		type SessionKey = UintAuthorityId;
		type InherentOfflineReport = ();
	}
	impl system::Trait for Test {
		type Origin = Origin;
		type Index = u64;
		type BlockNumber = u64;
		type Hash = H256;
		type Hashing = BlakeTwo256;
		type Digest = Digest;
		type AccountId = u64;
		type Lookup = IdentityLookup<Self::AccountId>;
		type Header = Header;
		type Event = ();
		type Log = DigestItem;
	}
	impl timestamp::Trait for Test {
		type Moment = u64;
		type OnTimestampSet = ();
	}
	impl Trait for Test {
		type ConvertAccountIdToSessionKey = ConvertUintAuthorityId;
		type OnSessionChange = TestOnSessionChange;
		type Event = ();
	}

	type System = system::Module<Test>;
	type Consensus = consensus::Module<Test>;
	type Session = Module<Test>;

	fn new_test_ext() -> runtime_io::TestExternalities<Blake2Hasher> {
		let mut t = system::GenesisConfig::<Test>::default().build_storage().unwrap().0;
		t.extend(consensus::GenesisConfig::<Test>{
			code: vec![],
			authorities: NEXT_VALIDATORS.with(|l| l.borrow().iter().cloned().map(UintAuthorityId).collect()),
		}.build_storage().unwrap().0);
		t.extend(timestamp::GenesisConfig::<Test>{
			minimum_period: 5,
		}.build_storage().unwrap().0);
		t.extend(GenesisConfig::<Test>{
			session_length: 2,
			validators: NEXT_VALIDATORS.with(|l| l.borrow().clone()),
			keys: vec![],
		}.build_storage().unwrap().0);
		runtime_io::TestExternalities::new(t)
	}

	#[test]
	fn simple_setup_should_work() {
		with_externalities(&mut new_test_ext(), || {
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2), UintAuthorityId(3)]);
			assert_eq!(Session::length(), 2);
			assert_eq!(Session::validators(), vec![1, 2, 3]);
		});
	}

	#[test]
	fn authorities_should_track_validators() {
		with_externalities(&mut new_test_ext(), || {
			NEXT_VALIDATORS.with(|v| *v.borrow_mut() = vec![1, 2]);
			assert_ok!(Session::force_new_session(false));
			Session::check_rotate_session(1);
			assert_eq!(Session::validators(), vec![1, 2]);
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2)]);

			NEXT_VALIDATORS.with(|v| *v.borrow_mut() = vec![1, 2, 4]);
			assert_ok!(Session::force_new_session(false));
			Session::check_rotate_session(2);
			assert_eq!(Session::validators(), vec![1, 2, 4]);
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2), UintAuthorityId(4)]);

			NEXT_VALIDATORS.with(|v| *v.borrow_mut() = vec![1, 2, 3]);
			assert_ok!(Session::force_new_session(false));
			Session::check_rotate_session(3);
			assert_eq!(Session::validators(), vec![1, 2, 3]);
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2), UintAuthorityId(3)]);
		});
	}

	#[test]
	fn should_work_with_early_exit() {
		with_externalities(&mut new_test_ext(), || {
			System::set_block_number(1);
			assert_ok!(Session::set_length(10));
			assert_eq!(Session::blocks_remaining(), 1);
			Session::check_rotate_session(1);

			System::set_block_number(2);
			assert_eq!(Session::blocks_remaining(), 0);
			Session::check_rotate_session(2);
			assert_eq!(Session::length(), 10);

			System::set_block_number(7);
			assert_eq!(Session::current_index(), 1);
			assert_eq!(Session::blocks_remaining(), 5);
			assert_ok!(Session::force_new_session(false));
			Session::check_rotate_session(7);

			System::set_block_number(8);
			assert_eq!(Session::current_index(), 2);
			assert_eq!(Session::blocks_remaining(), 9);
			Session::check_rotate_session(8);

			System::set_block_number(17);
			assert_eq!(Session::current_index(), 2);
			assert_eq!(Session::blocks_remaining(), 0);
			Session::check_rotate_session(17);

			System::set_block_number(18);
			assert_eq!(Session::current_index(), 3);
		});
	}

	#[test]
	fn session_length_change_should_work() {
		with_externalities(&mut new_test_ext(), || {
			// Block 1: Change to length 3; no visible change.
			System::set_block_number(1);
			assert_ok!(Session::set_length(3));
			Session::check_rotate_session(1);
			assert_eq!(Session::length(), 2);
			assert_eq!(Session::current_index(), 0);

			// Block 2: Length now changed to 3. Index incremented.
			System::set_block_number(2);
			assert_ok!(Session::set_length(3));
			Session::check_rotate_session(2);
			assert_eq!(Session::length(), 3);
			assert_eq!(Session::current_index(), 1);

			// Block 3: Length now changed to 3. Index incremented.
			System::set_block_number(3);
			Session::check_rotate_session(3);
			assert_eq!(Session::length(), 3);
			assert_eq!(Session::current_index(), 1);

			// Block 4: Change to length 2; no visible change.
			System::set_block_number(4);
			assert_ok!(Session::set_length(2));
			Session::check_rotate_session(4);
			assert_eq!(Session::length(), 3);
			assert_eq!(Session::current_index(), 1);

			// Block 5: Length now changed to 2. Index incremented.
			System::set_block_number(5);
			Session::check_rotate_session(5);
			assert_eq!(Session::length(), 2);
			assert_eq!(Session::current_index(), 2);

			// Block 6: No change.
			System::set_block_number(6);
			Session::check_rotate_session(6);
			assert_eq!(Session::length(), 2);
			assert_eq!(Session::current_index(), 2);

			// Block 7: Next index.
			System::set_block_number(7);
			Session::check_rotate_session(7);
			assert_eq!(Session::length(), 2);
			assert_eq!(Session::current_index(), 3);
		});
	}

	#[test]
	fn session_change_should_work() {
		with_externalities(&mut new_test_ext(), || {
			// Block 1: No change
			System::set_block_number(1);
			Session::check_rotate_session(1);
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2), UintAuthorityId(3)]);

			// Block 2: Session rollover, but no change.
			System::set_block_number(2);
			Session::check_rotate_session(2);
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2), UintAuthorityId(3)]);

			// Block 3: Set new key for validator 2; no visible change.
			System::set_block_number(3);
			assert_ok!(Session::set_key(Origin::signed(2), UintAuthorityId(5)));
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2), UintAuthorityId(3)]);

			Session::check_rotate_session(3);
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(2), UintAuthorityId(3)]);

			// Block 4: Session rollover, authority 2 changes.
			System::set_block_number(4);
			Session::check_rotate_session(4);
			assert_eq!(Consensus::authorities(), vec![UintAuthorityId(1), UintAuthorityId(5), UintAuthorityId(3)]);
		});
	}
}
