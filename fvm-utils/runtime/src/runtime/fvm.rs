// Copyright 2022-2024 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
use anyhow::Error;
use cid::multihash::{Code, MultihashDigest};
use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{to_vec, CborStore, DAG_CBOR};
use fvm_sdk as fvm;
use fvm_sdk::NO_DATA_BLOCK_ID;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum};
use num_traits::Zero;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::runtime::actor_blockstore::ActorBlockstore;
use crate::runtime::{ActorCode, MessageInfo, Primitives};
use crate::{actor_error, deserialize_block, ActorError, Runtime, Type};

pub const PUBKEY_ADDRESS_METHOD: u64 = 2;
// The original method is `2`, but we have a custom account actor
// with a public function for public address resolution so it can be
// called from non-builtin actors
pub const PUBLIC_RESOLVE_ADDRESS_METHOD: u64 = frc42_dispatch::method_hash!("ResolvePubKeyAddress");

lazy_static! {
    /// Cid of the empty array Cbor bytes (`EMPTY_ARR_BYTES`).
    pub static ref EMPTY_ARR_CID: Cid = {
        let empty = to_vec::<[(); 0]>(&[]).unwrap();
        Cid::new_v1(DAG_CBOR, Code::Blake2b256.digest(&empty))
    };
}

/// A runtime that bridges to the FVM environment through the FVM SDK.
pub struct FvmRuntime<B = ActorBlockstore> {
    blockstore: B,
    /// Indicates whether we are in a state transaction. During such, sending
    /// messages is prohibited.
    in_transaction: bool,
    /// Indicates that the caller has been validated.
    caller_validated: bool,
}

impl Default for FvmRuntime {
    fn default() -> Self {
        FvmRuntime {
            blockstore: ActorBlockstore,
            in_transaction: false,
            caller_validated: false,
        }
    }
}

impl<B> FvmRuntime<B> {
    fn assert_not_validated(&mut self) -> Result<(), ActorError> {
        if self.caller_validated {
            return Err(actor_error!(
                assertion_failed,
                "Method must validate caller identity exactly once"
            ));
        }
        Ok(())
    }
}

/// A stub MessageInfo implementation performing FVM syscalls to obtain its fields.
struct FvmMessage;

impl MessageInfo for FvmMessage {
    fn caller(&self) -> Address {
        Address::new_id(fvm::message::caller())
    }

    fn receiver(&self) -> Address {
        Address::new_id(fvm::message::receiver())
    }

    fn value_received(&self) -> TokenAmount {
        fvm::message::value_received()
    }
}

impl<B> Runtime for FvmRuntime<B>
where
    B: Blockstore,
{
    type Blockstore = B;

    fn network_version(&self) -> NetworkVersion {
        fvm::network::version()
    }

    fn message(&self) -> &dyn MessageInfo {
        &FvmMessage
    }

    fn curr_epoch(&self) -> ChainEpoch {
        fvm::network::curr_epoch()
    }

    fn validate_immediate_caller_accept_any(&mut self) -> Result<(), ActorError> {
        self.assert_not_validated()?;
        self.caller_validated = true;
        Ok(())
    }

    fn validate_immediate_caller_is<'a, I>(&mut self, addresses: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Address>,
    {
        self.assert_not_validated()?;
        let caller_addr = self.message().caller();
        if addresses.into_iter().any(|a| *a == caller_addr) {
            self.caller_validated = true;
            Ok(())
        } else {
            Err(actor_error!(forbidden;
                "caller {} is not one of supported", caller_addr
            ))
        }
    }

    fn validate_immediate_caller_type<'a, I>(&mut self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        self.assert_not_validated()?;
        let caller_cid = {
            let caller_addr = self.message().caller();
            self.get_actor_code_cid(&caller_addr.id().unwrap())
                .expect("failed to lookup caller code")
        };

        match self.resolve_builtin_actor_type(&caller_cid) {
            Some(typ) if types.into_iter().any(|t| *t == typ) => {
                self.caller_validated = true;
                Ok(())
            }
            _ => Err(actor_error!(forbidden;
                    "caller cid type {} not one of supported", caller_cid)),
        }
    }

    fn validate_immediate_caller_not_type<'a, I>(&mut self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        self.assert_not_validated()?;
        let caller_cid = {
            let caller_addr = self.message().caller();
            self.get_actor_code_cid(&caller_addr.id().unwrap())
                .expect("failed to lookup caller code")
        };

        match self.resolve_builtin_actor_type(&caller_cid) {
            Some(typ) if types.into_iter().any(|t| *t == typ) => Err(actor_error!(forbidden;
                                 "caller cid type {} is one of the not supported", caller_cid)),
            _ => {
                self.caller_validated = true;
                Ok(())
            }
        }
    }

    fn current_balance(&self) -> TokenAmount {
        fvm::sself::current_balance()
    }

    fn resolve_address(&self, address: &Address) -> Option<Address> {
        fvm::actor::resolve_address(address).map(Address::new_id)
    }

    fn get_actor_code_cid(&self, id: &ActorID) -> Option<Cid> {
        fvm::actor::get_actor_code_cid(&Address::new_id(*id))
    }

    fn create<T: Serialize>(&mut self, obj: &T) -> Result<(), ActorError> {
        let root = fvm::sself::root()?;
        if root != *EMPTY_ARR_CID {
            return Err(
                actor_error!(illegal_state; "failed to create state; expected empty array CID, got: {}", root),
            );
        }
        let new_root = ActorBlockstore.put_cbor(obj, Code::Blake2b256)
            .map_err(|e| actor_error!(illegal_argument; "failed to write actor state during creation: {}", e.to_string()))?;
        fvm::sself::set_root(&new_root)?;
        Ok(())
    }

    fn state<T: DeserializeOwned>(&self) -> Result<T, ActorError> {
        let root = fvm::sself::root()?;
        Ok(ActorBlockstore
            .get_cbor(&root)
            .map_err(|_| actor_error!(illegal_argument; "failed to get actor for Readonly state"))?
            .expect("State does not exist for actor state root"))
    }

    fn transaction<S, RT, F>(&mut self, f: F) -> Result<RT, ActorError>
    where
        S: Serialize + DeserializeOwned,
        F: FnOnce(&mut S, &mut Self) -> Result<RT, ActorError>,
    {
        let state_cid = fvm::sself::root()
            .map_err(|_| actor_error!(illegal_argument; "failed to get actor root state CID"))?;

        log::debug!("getting cid: {}", state_cid);

        let mut state = ActorBlockstore
            .get_cbor::<S>(&state_cid)
            .map_err(|_| actor_error!(illegal_argument; "failed to get actor state"))?
            .expect("State does not exist for actor state root");

        self.in_transaction = true;
        let result = f(&mut state, self);
        self.in_transaction = false;

        let ret = result?;
        let new_root = ActorBlockstore.put_cbor(&state, Code::Blake2b256)
            .map_err(|e| actor_error!(illegal_argument; "failed to write actor state in transaction: {}", e.to_string()))?;
        fvm::sself::set_root(&new_root)?;
        Ok(ret)
    }

    fn store(&self) -> &B {
        &self.blockstore
    }

    fn send(
        &self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
    ) -> Result<Option<IpldBlock>, ActorError> {
        if self.in_transaction {
            return Err(actor_error!(assertion_failed; "send is not allowed during transaction"));
        }
        match fvm::send::send(to, method, params, value, None, SendFlags::empty()) {
            Ok(ret) => {
                if ret.exit_code.is_success() {
                    Ok(ret.return_data)
                } else {
                    Err(ActorError::checked(
                        ret.exit_code,
                        format!(
                            "send to {} method {} aborted with code {}",
                            to, method, ret.exit_code
                        ),
                        ret.return_data,
                    ))
                }
            }
            Err(err) => Err(match err {
                // Some of these errors are from operations in the Runtime or SDK layer
                // before or after the underlying VM send syscall.
                ErrorNumber::NotFound => {
                    // This means that the receiving actor doesn't exist.
                    // TODO: we can't reasonably determine the correct "exit code" here.
                    actor_error!(unspecified; "receiver not found")
                }
                ErrorNumber::InsufficientFunds => {
                    // This means that the send failed because we have insufficient funds. We will
                    // get a _syscall error_, not an exit code, because the target actor will not
                    // run (and therefore will not exit).
                    actor_error!(insufficient_funds; "not enough funds")
                }
                ErrorNumber::LimitExceeded => {
                    // This means we've exceeded the recursion limit.
                    // TODO: Define a better exit code.
                    actor_error!(assertion_failed; "recursion limit exceeded")
                }
                err => {
                    // We don't expect any other syscall exit codes.
                    actor_error!(assertion_failed; "unexpected error: {}", err)
                }
            }),
        }
    }

    fn new_actor_address(&mut self) -> Result<Address, ActorError> {
        Ok(fvm::actor::next_actor_address())
    }

    fn create_actor(&mut self, code_id: Cid, actor_id: ActorID) -> Result<(), ActorError> {
        if self.in_transaction {
            return Err(
                actor_error!(assertion_failed; "create_actor is not allowed during transaction"),
            );
        }
        fvm::actor::create_actor(actor_id, &code_id, None).map_err(|e| match e {
            ErrorNumber::IllegalArgument => {
                ActorError::illegal_argument("failed to create actor".into())
            }
            _ => actor_error!(assertion_failed; "create failed with unknown error: {}", e),
        })
    }

    fn delete_actor(&mut self, burn_unspent: bool) -> Result<(), ActorError> {
        if self.in_transaction {
            return Err(
                actor_error!(assertion_failed; "delete_actor is not allowed during transaction"),
            );
        }
        Ok(fvm::sself::self_destruct(burn_unspent)?)
    }

    fn resolve_builtin_actor_type(&self, code_id: &Cid) -> Option<Type> {
        fvm::actor::get_builtin_actor_type(code_id).map(Type::from_i32)
    }

    fn get_code_cid_for_type(&self, typ: Type) -> Cid {
        fvm::actor::get_code_cid_for_type(typ as i32)
    }

    fn total_fil_circ_supply(&self) -> TokenAmount {
        fvm::network::total_fil_circ_supply()
    }

    fn charge_gas(&mut self, name: &'static str, compute: i64) {
        fvm::gas::charge(name, compute as u64)
    }

    fn base_fee(&self) -> TokenAmount {
        fvm::network::base_fee()
    }
}

impl<B> Primitives for FvmRuntime<B>
where
    B: Blockstore,
{
    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        fvm::crypto::hash_blake2b(data)
    }

    fn verify_signature(
        &self,
        signature: &Signature,
        signer: &Address,
        plaintext: &[u8],
    ) -> Result<(), Error> {
        match fvm::crypto::verify_signature(signature, signer, plaintext) {
            Ok(true) => Ok(()),
            Ok(false) | Err(_) => Err(Error::msg("invalid signature")),
        }
    }
}

/// A convenience function that built-in actors can delegate their execution to.
///
/// The trampoline takes care of boilerplate:
///
/// 0.  Initialize logging if debugging is enabled.
/// 1.  Obtains the parameter data from the FVM by fetching the parameters block.
/// 2.  Obtains the method number for the invocation.
/// 3.  Creates an FVM runtime shim.
/// 4.  Invokes the target method.
/// 5a. In case of error, aborts the execution with the emitted exit code, or
/// 5b. In case of success, stores the return data as a block and returns the latter.
pub fn trampoline<C: ActorCode>(params: u32) -> u32 {
    init_logging();

    std::panic::set_hook(Box::new(|info| {
        fvm::vm::abort(
            ExitCode::USR_ASSERTION_FAILED.value(),
            Some(&format!("{info}")),
        )
    }));

    let method = fvm::message::method_number();
    let params = fvm::message::params_raw(params).expect("params block invalid");

    // Construct a new runtime.
    let mut rt = FvmRuntime::default();
    // Invoke the method, aborting if the actor returns an errored exit code.
    let ret = C::invoke_method(&mut rt, method, params)
        .unwrap_or_else(|err| fvm::vm::abort(err.exit_code().value(), Some(err.msg())));

    // Abort with "assertion failed" if the actor failed to validate the caller somewhere.
    // We do this after handling the error, because the actor may have encountered an error before
    // it even could validate the caller.
    if !rt.caller_validated {
        fvm::vm::abort(
            ExitCode::USR_ASSERTION_FAILED.value(),
            Some("failed to validate caller"),
        )
    }

    // Then handle the return value.
    match ret {
        None => NO_DATA_BLOCK_ID,
        Some(ret_block) => fvm::ipld::put_block(ret_block.codec, ret_block.data.as_slice())
            .expect("failed to write result"),
    }
}

/// If debugging is enabled in the VM, installs a logger that sends messages to the FVM log syscall.
/// Messages are prefixed with "[LEVEL] ".
/// If debugging is not enabled, no logger will be installed which means that log!() and
/// similar calls will be dropped without either formatting args or making a syscall.
/// Note that, when debugging, the log syscalls will charge gas that wouldn't be charged
/// when debugging is not enabled.
///
/// Note: this is similar to fvm::debug::init_logging() from the FVM SDK, but
/// that doesn't work (at FVM SDK v2.2).
fn init_logging() {
    struct Logger;

    impl log::Log for Logger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            // Note the log system won't automatically call enabled() before this,
            // so it's canonical to check it here.
            // But logging must have been enabled at initialisation time in order for
            // the logger to be installed.
            // There's currently no use for dynamically disabling logging, so just skip checking.
            let msg = format!("[{}] {}", record.level(), record.args());
            fvm::debug::log(msg);
        }

        fn flush(&self) {}
    }
}

/// Resolves the SECP or BLS public key of an account actor ID address.
pub fn resolve_secp_bls(rt: &mut impl Runtime, addr: &Address) -> Result<Address, ActorError> {
    // return directly if it is already a public key
    match addr.protocol() {
        Protocol::Secp256k1 | Protocol::BLS => Ok(*addr),
        Protocol::ID => {
            let ret = rt.send(
                addr,
                PUBLIC_RESOLVE_ADDRESS_METHOD,
                None,
                TokenAmount::zero(),
            )?;
            deserialize_block(ret)
        }
        _ => Err(ActorError::illegal_argument(String::from(
            "address type not compatible",
        ))),
    }
}

pub fn equal_account_id(rt: &mut impl Runtime, a: &Address, b: &Address) -> bool {
    let a_id = match rt.resolve_address(a) {
        Some(id) => id,
        None => {
            return false;
        }
    };
    let b_id = match rt.resolve_address(b) {
        Some(id) => id,
        None => {
            return false;
        }
    };
    a_id == b_id
}
