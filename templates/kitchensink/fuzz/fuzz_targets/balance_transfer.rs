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
use pallet_balances::{Holds, TotalIssuance};
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

// Wrapper to implement Arbitrary for RuntimeCall (focusing on Balances for now)
#[derive(Debug)]
struct ArbitraryRuntimeCall(RuntimeCall);

impl<'a> Arbitrary<'a> for ArbitraryRuntimeCall {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Only generate Balances::transfer_allow_death calls
        let dest_idx: u8 = u.arbitrary()?;
        let amount: Balance = u.arbitrary()?;

        // Encode dest index in a temp account (will resolve to actual account during execution)
        let dest = [dest_idx; 32].into();

        Ok(ArbitraryRuntimeCall(
            RuntimeCall::Balances(pallet_balances::Call::transfer_allow_death {
                dest: sp_runtime::MultiAddress::Id(dest),
                value: amount,
            })
        ))
    }
}

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    origin_idx: u8,
    call: ArbitraryRuntimeCall,
}

fn minimal_genesis(accounts: &[AccountId]) -> Storage {
    const ENDOWMENT: Balance = 10_000_000 * DOLLARS;

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

fuzz_target!(|input: FuzzInput| {
    // Initialize logger to capture try_state errors (once per process)
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Error)
        .is_test(true)
        .try_init();

    // Set up 5 test accounts
    let accounts: Vec<AccountId> = (0..5).map(|i| [i; 32].into()).collect();
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

        let after_init = pallet_balances::TotalIssuance::<Runtime>::get();

        // Set timestamp (required for block finalization)
        Timestamp::set(RuntimeOrigin::none(), u64::from(block) * SLOT_DURATION).unwrap();

        let after_timestamp = pallet_balances::TotalIssuance::<Runtime>::get();

        // Pick origin from accounts
        let origin = accounts[input.origin_idx as usize % accounts.len()].clone();

        // Execute the transfer
        let _result = input.call.0.dispatch(RuntimeOrigin::signed(origin));

        let after_dispatch = pallet_balances::TotalIssuance::<Runtime>::get();

        // Finalize block
        Executive::finalize_block();

        let after_finalize = pallet_balances::TotalIssuance::<Runtime>::get();

        // Debug: print when issuance changes
        if initial_issuance != after_init {
            eprintln!("Issuance changed during initialize_block: {} -> {}", initial_issuance, after_init);
        }
        if after_init != after_timestamp {
            eprintln!("Issuance changed during timestamp set: {} -> {}", after_init, after_timestamp);
        }
        if after_timestamp != after_dispatch {
            eprintln!("Issuance changed during dispatch: {} -> {}", after_timestamp, after_dispatch);
        }
        if after_dispatch != after_finalize {
            eprintln!("Issuance changed during finalize: {} -> {}", after_dispatch, after_finalize);
        }

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

        let total_issuance = after_finalize;
        let counted_issuance = counted_free + counted_reserved;

        // Issuance must equal sum of all balances
        assert_eq!(total_issuance, counted_issuance,
            "Total issuance mismatch: recorded={}, counted={} (free={}, reserved={})",
            total_issuance, counted_issuance, counted_free, counted_reserved);

        // Issuance can only decrease, never increase
        assert!(total_issuance <= initial_issuance,
            "Issuance increased! initial={}, final={}, diff=+{}",
            initial_issuance, total_issuance, total_issuance - initial_issuance);

        // Run developer-defined integrity tests
        AllPalletsWithSystem::integrity_test();

        // Run try_state checks for all pallets
        if let Err(e) = AllPalletsWithSystem::try_state(block, TryStateSelect::All) {
            eprintln!("try_state failed: {:?}", e);
            panic!("try_state check failed: {:?}", e);
        }
    });
});
