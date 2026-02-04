#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use kitchensink_runtime::{
    constants::{currency::DOLLARS, time::SLOT_DURATION},
    AccountId, AllPalletsWithSystem, Balances, Executive, Runtime, RuntimeCall, RuntimeOrigin,
    RuntimeGenesisConfig, BalancesConfig, SystemConfig, Timestamp,
    BeefyConfig, SessionConfig, SessionKeys,
};
use frame_system::Account;
use frame_support::traits::{IntegrityTest, TryState, TryStateSelect};
use pallet_balances::Holds;
use node_primitives::Balance;
use sp_runtime::{
    traits::{Dispatchable, Header as HeaderT},
    testing::H256,
    Digest, DigestItem, BuildStorage, Storage,
};
use sp_consensus_babe::{
    digests::{PreDigest, SecondaryPlainPreDigest},
    Slot, BABE_ENGINE_ID,
    AuthorityId as BabeId,
};
use codec::Encode;
use pallet_grandpa::AuthorityId as GrandpaId;
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use sp_authority_discovery::AuthorityId as AuthorityDiscoveryId;
use sp_core::{sr25519::Public as MixnetId, Pair};
use sp_runtime::app_crypto::ByteArray;
use sp_state_machine::BasicExternalities;
use libfuzzer_sys::fuzz_target;

const GENESIS_ACCOUNTS: u8 = 100;
const ENDOWMENT: Balance = 10_000_000 * DOLLARS;

#[derive(Debug)]
enum CallOrigin {
    Signed(u8), // account index
    Root,
}

// Wrapper to implement Arbitrary for RuntimeCall with its required origin
#[derive(Debug)]
struct ArbitraryRuntimeCall {
    call: RuntimeCall,
    origin: CallOrigin,
}

impl<'a> Arbitrary<'a> for ArbitraryRuntimeCall {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let call_type: u8 = u.int_in_range(0..=4)?;

        let (call, origin) = match call_type {
            0 => {
                // transfer_allow_death
                let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let amount: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
                (
                    RuntimeCall::Balances(pallet_balances::Call::transfer_allow_death {
                        dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
                        value: amount,
                    }),
                    CallOrigin::Signed(origin_idx),
                )
            }
            1 => {
                // transfer_keep_alive
                let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let amount: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
                (
                    RuntimeCall::Balances(pallet_balances::Call::transfer_keep_alive {
                        dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
                        value: amount,
                    }),
                    CallOrigin::Signed(origin_idx),
                )
            }
            2 => {
                // transfer_all
                let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let keep_alive: bool = u.arbitrary()?;
                (
                    RuntimeCall::Balances(pallet_balances::Call::transfer_all {
                        dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
                        keep_alive,
                    }),
                    CallOrigin::Signed(origin_idx),
                )
            }
            3 => {
                // force_transfer (requires root)
                let source_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let amount: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
                (
                    RuntimeCall::Balances(pallet_balances::Call::force_transfer {
                        source: sp_runtime::MultiAddress::Id([source_idx; 32].into()),
                        dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
                        value: amount,
                    }),
                    CallOrigin::Root,
                )
            }
            4 => {
                // force_set_balance (requires root)
                let who_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
                let new_free: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
                (
                    RuntimeCall::Balances(pallet_balances::Call::force_set_balance {
                        who: sp_runtime::MultiAddress::Id([who_idx; 32].into()),
                        new_free,
                    }),
                    CallOrigin::Root,
                )
            }
            _ => unreachable!(),
        };

        Ok(ArbitraryRuntimeCall { call, origin })
    }
}

fn minimal_genesis(accounts: &[AccountId]) -> Storage {
    let beefy_pair = sp_consensus_beefy::ecdsa_crypto::Pair::generate().0;

    RuntimeGenesisConfig {
        system: SystemConfig::default(),
        balances: BalancesConfig {
            balances: accounts.iter().cloned().map(|x| (x, ENDOWMENT)).collect(),
            dev_accounts: None,
        },
        session: SessionConfig {
            keys: vec![(
                [0; 32].into(),
                [0; 32].into(),
                SessionKeys {
                    grandpa: GrandpaId::from_slice(&[0; 32]).unwrap(),
                    babe: BabeId::from_slice(&[0; 32]).unwrap(),
                    beefy: beefy_pair.public(),
                    im_online: ImOnlineId::from_slice(&[0; 32]).unwrap(),
                    authority_discovery: AuthorityDiscoveryId::from_slice(&[0; 32]).unwrap(),
                    mixnet: MixnetId::from_slice(&[0; 32]).unwrap().into(),
                },
            )],
            non_authority_keys: vec![],
        },
        beefy: BeefyConfig::default(),
        ..Default::default()
    }
    .build_storage()
    .unwrap()
}

fuzz_target!(|input: ArbitraryRuntimeCall| {
    // Initialize logger to capture try_state errors (once per process)
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Error)
        .is_test(true)
        .try_init();

    // Set up genesis test accounts
    let accounts: Vec<AccountId> = (0..GENESIS_ACCOUNTS).map(|i| [i; 32].into()).collect();
    let genesis = minimal_genesis(&accounts);

    BasicExternalities::execute_with_storage(&mut genesis.clone(), || {
        // Capture initial issuance
        let initial_issuance = pallet_balances::TotalIssuance::<Runtime>::get();

        // Initialize block 1
        let block = 1u32;
        let pre_digest = Digest {
            logs: vec![DigestItem::PreRuntime(
                BABE_ENGINE_ID,
                PreDigest::SecondaryPlain(SecondaryPlainPreDigest {
                    slot: Slot::from(u64::from(block)),
                    authority_index: 42,
                })
                .encode(),
            )],
        };

        type Header = sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256>;
        Executive::initialize_block(&Header::new(
            block,
            H256::default(),
            H256::default(),
            H256::default(),
            pre_digest,
        ));

        // Set timestamp (required for block finalization)
        Timestamp::set(RuntimeOrigin::none(), u64::from(block) * SLOT_DURATION).unwrap();

        // Determine runtime origin based on call requirements
        let runtime_origin = match input.origin {
            CallOrigin::Signed(idx) => {
                let account = accounts[idx as usize % accounts.len()].clone();
                RuntimeOrigin::signed(account)
            }
            CallOrigin::Root => RuntimeOrigin::root(),
        };

        // Execute the call
        let _result = input.call.dispatch(runtime_origin);

        // Finalize block
        Executive::finalize_block();

        let final_issuance = pallet_balances::TotalIssuance::<Runtime>::get();

        // Check all invariants (matching kitchensink fuzzer)
        let mut counted_free: Balance = 0;
        let mut counted_reserved: Balance = 0;

        for (account, info) in Account::<Runtime>::iter() {
            let consumers = info.consumers;
            let providers = info.providers;
            assert!(!(consumers > 0 && providers == 0),
                "Invalid consumer/provider state for account {:?}: consumers={}, providers={}",
                account, consumers, providers);

            counted_free += info.data.free;
            counted_reserved += info.data.reserved;

            // Check max lock equals frozen balance
            let max_lock: Balance = Balances::locks(&account)
                .iter()
                .map(|l| l.amount)
                .max()
                .unwrap_or_default();
            assert_eq!(max_lock, info.data.frozen,
                "Max lock should equal frozen balance for {:?}: max_lock={}, frozen={}",
                account, max_lock, info.data.frozen);

            // Check sum of holds <= reserved
            let sum_holds: Balance = Holds::<Runtime>::get(&account)
                .iter()
                .map(|l| l.amount)
                .sum();
            assert!(sum_holds <= info.data.reserved,
                "Sum of holds ({}) exceeds reserved balance ({}) for {:?}",
                sum_holds, info.data.reserved, account);
        }

        let counted_issuance = counted_free + counted_reserved;

        // Issuance must equal sum of all balances
        assert_eq!(final_issuance, counted_issuance,
            "Total issuance mismatch: recorded={}, counted={} (free={}, reserved={})",
            final_issuance, counted_issuance, counted_free, counted_reserved);

        // Run developer-defined integrity tests
        AllPalletsWithSystem::integrity_test();

        // Run try_state checks for all pallets
        if let Err(e) = AllPalletsWithSystem::try_state(block, TryStateSelect::All) {
            eprintln!("try_state failed: {:?}", e);
            panic!("try_state check failed: {:?}", e);
        }
    });
});
