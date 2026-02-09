#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use kitchensink_runtime::{
    constants::{currency::DOLLARS, time::SLOT_DURATION},
    AccountId, AllPalletsWithSystem, Executive, RuntimeCall, RuntimeOrigin,
    RuntimeGenesisConfig, BalancesConfig, SystemConfig, Timestamp,
    BeefyConfig, SessionConfig, SessionKeys,
};
use frame_support::traits::{TryState, TryStateSelect};
use pallet_vesting::VestingInfo;
use node_primitives::{Balance, BlockNumber};
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
use libfuzzer_sys::fuzz_target;
use std::fs::OpenOptions;
use std::io::Write;
use sp_consensus_beefy::ecdsa_crypto;

const GENESIS_ACCOUNTS: u8 = 100;
const ENDOWMENT: Balance = 10_000_000 * DOLLARS;

#[derive(Debug)]
enum CallOrigin {
    Signed(u8),
    Root,
}

#[derive(Debug)]
struct ArbitraryRuntimeCall {
    call: RuntimeCall,
    origin: CallOrigin,
}

// Call generator specification
struct CallSpec {
    weight: u32,
    generator: fn(&mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)>,
}

fn gen_transfer_allow_death(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let amount: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
    Ok((
        RuntimeCall::Balances(pallet_balances::Call::transfer_allow_death {
            dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
            value: amount,
        }),
        CallOrigin::Signed(origin_idx),
    ))
}

fn gen_transfer_keep_alive(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let amount: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
    Ok((
        RuntimeCall::Balances(pallet_balances::Call::transfer_keep_alive {
            dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
            value: amount,
        }),
        CallOrigin::Signed(origin_idx),
    ))
}

fn gen_transfer_all(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let keep_alive: bool = u.arbitrary()?;
    Ok((
        RuntimeCall::Balances(pallet_balances::Call::transfer_all {
            dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
            keep_alive,
        }),
        CallOrigin::Signed(origin_idx),
    ))
}

fn gen_force_transfer(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let source_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let amount: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
    Ok((
        RuntimeCall::Balances(pallet_balances::Call::force_transfer {
            source: sp_runtime::MultiAddress::Id([source_idx; 32].into()),
            dest: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
            value: amount,
        }),
        CallOrigin::Root,
    ))
}

fn gen_force_set_balance(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let who_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let new_free: Balance = u.int_in_range(0..=(100 * ENDOWMENT))?;
    Ok((
        RuntimeCall::Balances(pallet_balances::Call::force_set_balance {
            who: sp_runtime::MultiAddress::Id([who_idx; 32].into()),
            new_free,
        }),
        CallOrigin::Root,
    ))
}

fn gen_vested_transfer(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let locked: Balance = u.int_in_range((100 * DOLLARS)..=(50 * ENDOWMENT))?;
    let per_block: Balance = u.int_in_range(1..=(locked / 100).max(1))?;
    let starting_block: BlockNumber = u.int_in_range(1..=1000)?;
    Ok((
        RuntimeCall::Vesting(pallet_vesting::Call::vested_transfer {
            target: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
            schedule: VestingInfo::new(locked, per_block, starting_block),
        }),
        CallOrigin::Signed(origin_idx),
    ))
}

fn gen_vest(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let origin_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    Ok((
        RuntimeCall::Vesting(pallet_vesting::Call::vest {}),
        CallOrigin::Signed(origin_idx),
    ))
}

fn gen_force_vested_transfer(u: &mut Unstructured) -> arbitrary::Result<(RuntimeCall, CallOrigin)> {
    let source_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let dest_idx: u8 = u.int_in_range(0..=(GENESIS_ACCOUNTS - 1))?;
    let locked: Balance = u.int_in_range((100 * DOLLARS)..=(50 * ENDOWMENT))?;
    let per_block: Balance = u.int_in_range(1..=(locked / 100).max(1))?;
    let starting_block: BlockNumber = u.int_in_range(1..=1000)?;
    Ok((
        RuntimeCall::Vesting(pallet_vesting::Call::force_vested_transfer {
            source: sp_runtime::MultiAddress::Id([source_idx; 32].into()),
            target: sp_runtime::MultiAddress::Id([dest_idx; 32].into()),
            schedule: VestingInfo::new(locked, per_block, starting_block),
        }),
        CallOrigin::Root,
    ))
}

// Call registry
const CALL_SPECS: &[CallSpec] = &[
    CallSpec { weight: 10, generator: gen_transfer_allow_death },
    CallSpec { weight: 10, generator: gen_transfer_keep_alive },
    CallSpec { weight: 5, generator: gen_transfer_all },
    CallSpec { weight: 2, generator: gen_force_transfer },
    CallSpec { weight: 1, generator: gen_force_set_balance },
    CallSpec { weight: 8, generator: gen_vested_transfer },
    CallSpec { weight: 5, generator: gen_vest },
    CallSpec { weight: 2, generator: gen_force_vested_transfer },
];

impl<'a> Arbitrary<'a> for ArbitraryRuntimeCall {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let total_weight: u32 = CALL_SPECS.iter().map(|s| s.weight).sum();
        let rand_value: u32 = u.int_in_range(0..=u32::MAX)?;
        let mut threshold = ((rand_value as u64 * total_weight as u64) / u32::MAX as u64) as u32;

        for spec in CALL_SPECS {
            if threshold < spec.weight {
                let (call, origin) = (spec.generator)(u)?;
                return Ok(ArbitraryRuntimeCall { call, origin });
            }
            threshold -= spec.weight;
        }

        // Fallback
        let (call, origin) = (CALL_SPECS[0].generator)(u)?;
        Ok(ArbitraryRuntimeCall { call, origin })
    }
}

// Generic newtype wrapper to ensure a Vec is never empty
#[derive(Debug)]
struct NonEmpty<T>(Vec<T>);

impl<'a, T: Arbitrary<'a>> Arbitrary<'a> for NonEmpty<T> {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let mut items: Vec<T> = u.arbitrary()?;
        if items.is_empty() {
            items.push(u.arbitrary()?);
        }
        Ok(NonEmpty(items))
    }
}

// Top-level structure: Vec<Vec<Call>> representing multiple blocks
#[derive(Debug)]
struct MultiBlockCalls(Vec<NonEmpty<ArbitraryRuntimeCall>>);

impl<'a> Arbitrary<'a> for MultiBlockCalls {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let blocks: Vec<NonEmpty<ArbitraryRuntimeCall>> = u.arbitrary()?;
        if blocks.is_empty() {
            let single_block: NonEmpty<ArbitraryRuntimeCall> = u.arbitrary()?;
            Ok(MultiBlockCalls(vec![single_block]))
        } else {
            Ok(MultiBlockCalls(blocks))
        }
    }
}

fn genesis_config() -> RuntimeGenesisConfig {
    let endowed_accounts: Vec<AccountId> = (0..GENESIS_ACCOUNTS)
        .map(|i| AccountId::from([i; 32]))
        .collect();

    // Generate beefy keypair for proper initialization
    let beefy_pair = ecdsa_crypto::Pair::generate().0;

    RuntimeGenesisConfig {
        system: SystemConfig::default(),
        balances: BalancesConfig {
            balances: endowed_accounts
                .iter()
                .map(|k| (k.clone(), ENDOWMENT))
                .collect(),
            dev_accounts: None,
        },
        session: SessionConfig {
            keys: vec![(
                [0; 32].into(),  // account
                [0; 32].into(),  // stash
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
}

fn create_block_builder_with_genesis() -> (Storage, RuntimeGenesisConfig) {
    let genesis_config = genesis_config();
    let storage = genesis_config.build_storage().expect("Storage should build");
    (storage, genesis_config)
}

fn initialize_block(block_number: u32, parent_hash: H256) {
    let slot = Slot::from(block_number as u64);
    let pre_digest = Digest {
        logs: vec![DigestItem::PreRuntime(
            BABE_ENGINE_ID,
            PreDigest::SecondaryPlain(SecondaryPlainPreDigest {
                authority_index: 0,
                slot,
            })
            .encode(),
        )],
    };

    let header = sp_runtime::generic::Header::<u32, sp_runtime::traits::BlakeTwo256>::new(
        block_number,
        Default::default(),
        Default::default(),
        parent_hash,
        pre_digest,
    );

    Executive::initialize_block(&header);
    Timestamp::set_timestamp((block_number as u64) * SLOT_DURATION);
}

fn finalize_block(_block_number: u32) -> H256 {
    let header = Executive::finalize_block();
    H256::from_slice(header.hash().as_ref())
}

fuzz_target!(|blocks: MultiBlockCalls| {
    env_logger::try_init().ok();

    let (mut storage, _genesis_config) = create_block_builder_with_genesis();
    let mut parent_hash = H256::default();

    if let Ok(mut log_file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("runtimes.log")
    {
        let _ = writeln!(log_file, "\n{}", "=".repeat(80));
        let _ = writeln!(log_file, "NEW FUZZER INPUT - {} blocks", blocks.0.len());
        let _ = writeln!(log_file, "{}", "=".repeat(80));

        for (block_idx, block_calls) in blocks.0.iter().enumerate() {
            let _ = writeln!(log_file, "\nBlock {}:", block_idx + 1);
            for (call_idx, call) in block_calls.0.iter().enumerate() {
                let origin_str = match &call.origin {
                    CallOrigin::Signed(idx) => format!("Signed(account_{})", idx),
                    CallOrigin::Root => "Root".to_string(),
                };
                let _ = writeln!(log_file, "  Call {}: {:?} (origin: {})", call_idx + 1, call.call, origin_str);
            }
        }
    }

    for (block_num, block) in blocks.0.iter().enumerate() {
        let block_number = (block_num + 1) as u32;

        sp_state_machine::BasicExternalities::execute_with_storage(&mut storage, || {
            initialize_block(block_number, parent_hash);

            for (call_idx, call) in block.0.iter().enumerate() {
                let runtime_origin = match call.origin {
                    CallOrigin::Signed(account_idx) => {
                        RuntimeOrigin::signed(AccountId::from([account_idx; 32]))
                    }
                    CallOrigin::Root => RuntimeOrigin::root(),
                };

                let result = call.call.clone().dispatch(runtime_origin);

                if let Ok(mut log_file) = OpenOptions::new()
                    .append(true)
                    .open("runtimes.log")
                {
                    let result_str = match &result {
                        Ok(_) => "✓ Ok".to_string(),
                        Err(e) => format!("✗ Err({:?})", e.error),
                    };
                    let _ = writeln!(log_file, "    → Block {} Call {} result: {}", block_num, call_idx + 1, result_str);
                }
            }

            parent_hash = finalize_block(block_number);
        });

        sp_state_machine::BasicExternalities::execute_with_storage(&mut storage, || {
            AllPalletsWithSystem::try_state(block_number as u32, TryStateSelect::All).unwrap();
        });
    }
});
