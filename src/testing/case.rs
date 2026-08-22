//! Argent-level test sidecars and deterministic `TxContext` synthesis.
//!
//! The schema contains authored actors, source state, entry calls, outputs, and
//! expectations. Concrete outpoints, covenant ids, UTXOs, generated arguments,
//! and Silver selectors never enter the test file.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::Path,
};

use blake2b_simd::Params as Blake2bParams;
use kaspa_consensus_core::{
    Hash,
    hashing::{
        sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash},
        sighash_type::SIG_HASH_ALL,
    },
    tx::{CovenantBinding, MutableTransaction, Transaction, TransactionId, TransactionOutpoint},
};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use serde_json::{Map, Value};

use crate::{
    artifact::{ActorArtifact, ArgentFieldArtifact, Artifact, EntryArtifact, SilContractArtifact, SourceArrayArtifact, TypeArtifact},
    builder::{ActorPath, ArgValue, ArtifactValue, EntryCall, TxBuilder, TxContext},
};

use super::{ArgentTestError, ArgentTestRunner, TestResult};

const DEFAULT_ACTOR_VALUE: u64 = 1_000;
const TEST_HASH_DOMAIN: &[u8] = b"argent/test-context/v1";

#[derive(Clone)]
struct TestKey([u8; 32]);

impl fmt::Debug for TestKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TestKey([redacted])")
    }
}

/// One parsed Argent test sidecar.
#[derive(Clone, Debug)]
pub struct TestFile {
    pub tests: Vec<TestCase>,
}

impl TestFile {
    /// Read and strictly validate an Argent `.test.json` sidecar.
    pub fn from_path(path: impl AsRef<Path>) -> TestResult<Self> {
        load_test_file(path).map_err(|error| ArgentTestError::Definition(error.to_string()))
    }
}

/// Expected transaction-level result for one test case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestExpectation {
    Accept,
    Reject(Option<String>),
}

/// One authored Argent transaction test.
#[derive(Clone, Debug)]
pub struct TestCase {
    pub name: String,
    pub expect: TestExpectation,
    inputs: Vec<TestInput>,
    outputs: Vec<TestOutput>,
    keys: BTreeMap<String, TestKey>,
}

impl TestCase {
    /// Lower the authored case into an owned runtime transaction context.
    pub fn build_context(&self, runner: &ArgentTestRunner, builder: &TxBuilder<'_>) -> TestResult<TxContext<'static>> {
        self.try_build_context(runner, builder).map_err(|error| ArgentTestError::Definition(error.to_string()))
    }

    fn try_build_context(&self, runner: &ArgentTestRunner, builder: &TxBuilder<'_>) -> Result<TxContext<'static>, CaseError> {
        let covenant_ids = self
            .inputs
            .iter()
            .filter_map(|input| input.covenant.as_ref())
            .map(|label| (label.clone(), derive_hash(&self.name, "named-covenant", label)))
            .collect::<BTreeMap<_, _>>();
        let input_covenant_ids = self
            .inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                input
                    .covenant
                    .as_ref()
                    .map_or_else(|| derive_hash(&self.name, "input-covenant", &index.to_string()), |label| covenant_ids[label])
            })
            .collect::<Vec<_>>();
        let values = ValueEnvironment { builder, keys: &self.keys, covenant_ids: &covenant_ids };
        let mut resolved_inputs = Vec::with_capacity(self.inputs.len());

        for (input_index, input) in self.inputs.iter().enumerate() {
            let loaded = runner
                .loaded_app_for_actor(&input.actor)
                .ok_or_else(|| CaseError::in_case(&self.name, format!("no compiled app provides actor path `{}`", input.actor)))?;
            let artifact = &loaded.artifact;
            let actor = actor_artifact(artifact, &input.actor)
                .map_err(|message| CaseError::in_case(&self.name, format!("input {input_index}: {message}")))?;
            let entry = entry_artifact(actor, &input.entry).ok_or_else(|| {
                CaseError::in_case(&self.name, format!("input {input_index}: unknown entry `{}::{}`", input.actor, input.entry))
            })?;
            if !entry.spawns.is_empty() {
                return Err(CaseError::in_case(
                    &self.name,
                    format!(
                        "input {input_index} `{}::{}` declares spawned genesis outputs; this sidecar version supports existing-covenant actor outputs only",
                        input.actor, input.entry
                    ),
                ));
            }
            let contract = artifact.sil_abi.contract(&actor.abi.actor).ok_or_else(|| {
                CaseError::in_case(
                    &self.name,
                    format!("input {input_index}: actor `{}` has no Sil ABI contract `{}`", input.actor, actor.abi.actor),
                )
            })?;
            let state = convert_source_state(artifact, actor, &input.state, &values, &format!("input {input_index} state"))
                .map_err(|message| CaseError::in_case(&self.name, message))?;
            let args =
                convert_entry_args(artifact, actor, entry, contract, &input.args, &values, &format!("input {input_index} args"))
                    .map_err(|message| CaseError::in_case(&self.name, message))?;
            resolved_inputs.push(ResolvedInput {
                actor: input.actor.clone(),
                state,
                entry: input.entry.clone(),
                args,
                value: input.value,
                covenant_id: input_covenant_ids[input_index],
                route_outputs: entry
                    .route_plan
                    .outputs
                    .iter()
                    .map(|output| output.actors.iter().map(|actor| route_actor_path(&input.actor, actor)).collect())
                    .collect(),
            });
        }

        let authorizers = derive_output_authorizers(runner, &self.name, &resolved_inputs, &self.outputs)?;
        let mut context = TxContext::new();
        for (input_index, input) in resolved_inputs.iter().enumerate() {
            let outpoint_hash = derive_hash(&self.name, "outpoint", &input_index.to_string());
            let outpoint = TransactionOutpoint::new(TransactionId::from_bytes(outpoint_hash.as_bytes()), 0);
            let utxo = builder
                .covenant_utxo(input.actor.clone(), input.state.clone(), input.value, 0, false, Some(input.covenant_id))
                .map_err(|error| {
                    CaseError::in_case(&self.name, format!("input {input_index}: failed to synthesize actor UTXO: {error}"))
                })?;
            context = context.actor_input(
                input.actor.clone(),
                input.state.clone(),
                entry_call(input.entry.clone(), input.args.clone()),
                outpoint,
                utxo,
                0,
            );
        }

        for (output_index, (output, &authorizer)) in self.outputs.iter().zip(&authorizers).enumerate() {
            let loaded = runner.loaded_app_for_actor(&output.actor).ok_or_else(|| {
                CaseError::in_case(&self.name, format!("no compiled app provides output actor path `{}`", output.actor))
            })?;
            let artifact = &loaded.artifact;
            let actor = actor_artifact(artifact, &output.actor)
                .map_err(|message| CaseError::in_case(&self.name, format!("output {output_index}: {message}")))?;
            let state = convert_source_state(artifact, actor, &output.state, &values, &format!("output {output_index} state"))
                .map_err(|message| CaseError::in_case(&self.name, message))?;
            let authorizing_input = u16::try_from(authorizer)
                .map_err(|_| CaseError::in_case(&self.name, format!("input index {authorizer} does not fit a covenant binding")))?;
            let binding = CovenantBinding::new(authorizing_input, resolved_inputs[authorizer].covenant_id);
            context = context.actor_output(output.actor.clone(), state, binding, output.value);
        }
        Ok(context)
    }
}

#[derive(Clone, Debug)]
struct TestInput {
    actor: ActorPath,
    state: Map<String, Value>,
    entry: String,
    args: Vec<Value>,
    value: u64,
    covenant: Option<String>,
}

#[derive(Clone, Debug)]
struct TestOutput {
    actor: ActorPath,
    state: Map<String, Value>,
    value: u64,
}

#[derive(Clone)]
enum TestArg {
    Static(ArgValue),
    Sign(TestKey),
}

struct ResolvedInput {
    actor: ActorPath,
    state: BTreeMap<String, ArtifactValue>,
    entry: String,
    args: Vec<TestArg>,
    value: u64,
    covenant_id: Hash,
    route_outputs: Vec<Vec<ActorPath>>,
}

struct ValueEnvironment<'a> {
    builder: &'a TxBuilder<'a>,
    keys: &'a BTreeMap<String, TestKey>,
    covenant_ids: &'a BTreeMap<String, Hash>,
}

#[derive(Debug)]
struct CaseError(String);

impl CaseError {
    fn at(context: impl AsRef<str>, message: impl Into<String>) -> Self {
        Self(format!("{}: {}", context.as_ref(), message.into()))
    }

    fn in_case(case: &str, message: impl Into<String>) -> Self {
        Self::at(format!("test `{case}`"), message)
    }
}

impl fmt::Display for CaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CaseError {}

#[derive(Debug)]
struct SigningError(String);

impl fmt::Display for SigningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SigningError {}

fn load_test_file(path: impl AsRef<Path>) -> Result<TestFile, CaseError> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|error| CaseError::at(path.display().to_string(), error.to_string()))?;
    let value =
        serde_json::from_str(&raw).map_err(|error| CaseError::at(path.display().to_string(), format!("invalid JSON: {error}")))?;
    parse_test_file(value, &path.display().to_string())
}

fn parse_test_file(value: Value, source: &str) -> Result<TestFile, CaseError> {
    let root = value.as_object().ok_or_else(|| CaseError::at(source, "test file must be a JSON object"))?;
    ensure_only_fields(root, &["keys", "tests"], source)?;
    let keys = root.get("keys").map(|value| parse_keys(value, &format!("{source}.keys"))).transpose()?.unwrap_or_default();
    let tests =
        required(root, "tests", source)?.as_array().ok_or_else(|| CaseError::at(format!("{source}.tests"), "expected an array"))?;
    let mut names = BTreeSet::new();
    let mut cases = Vec::with_capacity(tests.len());
    for (index, value) in tests.iter().enumerate() {
        let context = format!("{source}.tests[{index}]");
        let mut case = parse_case(value, &context)?;
        if !names.insert(case.name.clone()) {
            return Err(CaseError::at(context, format!("duplicate test name `{}`", case.name)));
        }
        case.keys = keys.clone();
        cases.push(case);
    }
    Ok(TestFile { tests: cases })
}

fn parse_keys(value: &Value, context: &str) -> Result<BTreeMap<String, TestKey>, CaseError> {
    let object = value.as_object().ok_or_else(|| CaseError::at(context, "expected an object of named test secrets"))?;
    object
        .iter()
        .map(|(name, value)| {
            if name.is_empty() {
                return Err(CaseError::at(context, "test key names must not be empty"));
            }
            let bytes = decode_hex(value, &format!("{context}.{name}"), Some(32)).map_err(CaseError)?;
            let bytes: [u8; 32] = bytes.try_into().expect("validated key length");
            SecretKey::from_slice(&bytes)
                .map_err(|error| CaseError::at(format!("{context}.{name}"), format!("invalid secp256k1 secret: {error}")))?;
            Ok((name.clone(), TestKey(bytes)))
        })
        .collect()
}

fn parse_case(value: &Value, context: &str) -> Result<TestCase, CaseError> {
    let object = value.as_object().ok_or_else(|| CaseError::at(context, "test case must be an object"))?;
    ensure_only_fields(object, &["name", "inputs", "outputs", "expect"], context)?;
    let name = nonempty_string(required(object, "name", context)?, &format!("{context}.name"))?;
    let inputs = parse_array(required(object, "inputs", context)?, &format!("{context}.inputs"), parse_input)?;
    if inputs.is_empty() {
        return Err(CaseError::at(format!("{context}.inputs"), "an Argent transaction test requires at least one actor input"));
    }
    let outputs = parse_array(required(object, "outputs", context)?, &format!("{context}.outputs"), parse_output)?;
    let expect = parse_expectation(required(object, "expect", context)?, &format!("{context}.expect"))?;
    Ok(TestCase { name, expect, inputs, outputs, keys: BTreeMap::new() })
}

fn parse_input(value: &Value, context: &str) -> Result<TestInput, CaseError> {
    let object = value.as_object().ok_or_else(|| CaseError::at(context, "input must be an object"))?;
    ensure_only_fields(object, &["actor", "state", "entry", "args", "value", "covenant"], context)?;
    let actor = parse_actor_path(required(object, "actor", context)?, &format!("{context}.actor"))?;
    let state = required(object, "state", context)?
        .as_object()
        .cloned()
        .ok_or_else(|| CaseError::at(format!("{context}.state"), "expected an object"))?;
    let entry = nonempty_string(required(object, "entry", context)?, &format!("{context}.entry"))?;
    let args = match object.get("args") {
        Some(value) => value.as_array().cloned().ok_or_else(|| CaseError::at(format!("{context}.args"), "expected an array"))?,
        None => Vec::new(),
    };
    let value = parse_value_amount(object.get("value"), &format!("{context}.value"))?;
    let covenant = object.get("covenant").map(|value| nonempty_string(value, &format!("{context}.covenant"))).transpose()?;
    Ok(TestInput { actor, state, entry, args, value, covenant })
}

fn parse_output(value: &Value, context: &str) -> Result<TestOutput, CaseError> {
    let object = value.as_object().ok_or_else(|| CaseError::at(context, "output must be an object"))?;
    ensure_only_fields(object, &["actor", "state", "value"], context)?;
    let actor = parse_actor_path(required(object, "actor", context)?, &format!("{context}.actor"))?;
    let state = required(object, "state", context)?
        .as_object()
        .cloned()
        .ok_or_else(|| CaseError::at(format!("{context}.state"), "expected an object"))?;
    let value = parse_value_amount(object.get("value"), &format!("{context}.value"))?;
    Ok(TestOutput { actor, state, value })
}

fn parse_expectation(value: &Value, context: &str) -> Result<TestExpectation, CaseError> {
    if let Some(value) = value.as_str() {
        return match value {
            "accept" => Ok(TestExpectation::Accept),
            "reject" => Ok(TestExpectation::Reject(None)),
            _ => Err(CaseError::at(context, "expected `accept`, `reject`, or `{\"reject\":\"Actor::entry\"}`")),
        };
    }
    let object =
        value.as_object().ok_or_else(|| CaseError::at(context, "expected `accept`, `reject`, or `{\"reject\":\"Actor::entry\"}`"))?;
    ensure_only_fields(object, &["reject"], context)?;
    let target = nonempty_string(required(object, "reject", context)?, &format!("{context}.reject"))?;
    let (actor, entry) = target
        .rsplit_once("::")
        .ok_or_else(|| CaseError::at(format!("{context}.reject"), "target must be `Actor::entry` or `app::Actor::entry`"))?;
    let actor = parse_actor_path(&Value::String(actor.to_string()), &format!("{context}.reject"))?;
    if entry.is_empty() || entry.contains("::") {
        return Err(CaseError::at(format!("{context}.reject"), "target must be `Actor::entry` or `app::Actor::entry`"));
    }
    Ok(TestExpectation::Reject(Some(format!("{actor}::{entry}"))))
}

fn parse_actor_path(value: &Value, context: &str) -> Result<ActorPath, CaseError> {
    let value = nonempty_string(value, context)?;
    let parts = value.split("::").collect::<Vec<_>>();
    if parts.iter().any(|part| part.is_empty()) || !(1..=2).contains(&parts.len()) {
        return Err(CaseError::at(context, "expected `Actor` or `app::Actor`"));
    }
    Ok(match parts.as_slice() {
        [actor] => ActorPath::primary(*actor),
        [app, actor] => ActorPath::qualified(super::app_alias(app), *actor),
        _ => unreachable!("actor path length was validated"),
    })
}

fn parse_value_amount(value: Option<&Value>, context: &str) -> Result<u64, CaseError> {
    match value {
        Some(value) => value.as_u64().ok_or_else(|| CaseError::at(context, "expected a non-negative integer")),
        None => Ok(DEFAULT_ACTOR_VALUE),
    }
}

fn parse_array<T>(value: &Value, context: &str, parse: impl Fn(&Value, &str) -> Result<T, CaseError>) -> Result<Vec<T>, CaseError> {
    let values = value.as_array().ok_or_else(|| CaseError::at(context, "expected an array"))?;
    values.iter().enumerate().map(|(index, value)| parse(value, &format!("{context}[{index}]"))).collect()
}

fn required<'a>(object: &'a Map<String, Value>, field: &str, context: &str) -> Result<&'a Value, CaseError> {
    object.get(field).ok_or_else(|| CaseError::at(context, format!("missing required field `{field}`")))
}

fn nonempty_string(value: &Value, context: &str) -> Result<String, CaseError> {
    let value = value.as_str().ok_or_else(|| CaseError::at(context, "expected a string"))?;
    if value.is_empty() {
        return Err(CaseError::at(context, "value must not be empty"));
    }
    Ok(value.to_string())
}

fn ensure_only_fields(object: &Map<String, Value>, allowed: &[&str], context: &str) -> Result<(), CaseError> {
    if let Some(field) = object.keys().find(|field| !allowed.contains(&field.as_str())) {
        return Err(CaseError::at(context, format!("unknown field `{field}`")));
    }
    Ok(())
}

fn actor_artifact<'a>(artifact: &'a Artifact, actor: &ActorPath) -> Result<&'a ActorArtifact, String> {
    artifact.argent.actors.iter().find(|candidate| candidate.name == actor.actor).ok_or_else(|| format!("unknown actor `{actor}`"))
}

fn entry_artifact<'a>(actor: &'a ActorArtifact, entry: &str) -> Option<&'a EntryArtifact> {
    actor.entries.iter().find(|candidate| candidate.name == entry)
}

fn convert_source_state(
    artifact: &Artifact,
    actor: &ActorArtifact,
    source: &Map<String, Value>,
    values: &ValueEnvironment<'_>,
    context: &str,
) -> Result<BTreeMap<String, ArtifactValue>, String> {
    if artifact.argent.states.iter().all(|state| state.name != actor.state) {
        return Err(format!("{context}: actor `{}` references unknown state `{}`", actor.name, actor.state));
    }
    convert_argent_state(artifact, &actor.state, source, values, context)
}

fn convert_argent_state(
    artifact: &Artifact,
    state_name: &str,
    source: &Map<String, Value>,
    values: &ValueEnvironment<'_>,
    context: &str,
) -> Result<BTreeMap<String, ArtifactValue>, String> {
    let state = artifact
        .argent
        .states
        .iter()
        .find(|state| state.name == state_name)
        .ok_or_else(|| format!("{context}: unknown Argent state `{state_name}`"))?;
    if let Some(extra) = source.keys().find(|name| state.fields.iter().all(|field| field.name != **name)) {
        return Err(format!("{context}: unknown field `{extra}`"));
    }
    state
        .fields
        .iter()
        .map(|field| {
            let value = source.get(&field.name).ok_or_else(|| format!("{context}: missing field `{}`", field.name))?;
            let field_context = format!("{context}.{}", field.name);
            let expanded_state = artifact
                .argent
                .state_expansions
                .iter()
                .find(|expansion| expansion.state == state.name)
                .and_then(|expansion| expansion.digests.iter().find(|digest| digest.field == field.name));
            let value = match expanded_state {
                Some(digest) => value
                    .as_object()
                    .ok_or_else(|| format!("{field_context}: expected an object of type `{}`", digest.state))
                    .and_then(|source| convert_argent_state(artifact, &digest.state, source, values, &field_context))
                    .map(ArtifactValue::Object),
                None if field.virtual_slot => {
                    Err(format!("{field_context}: virtual field has no authored expansion type in the artifact"))
                }
                None => convert_source_field(artifact, field, value, values, &field_context),
            }?;
            Ok((field.name.clone(), value))
        })
        .collect()
}

fn convert_source_field(
    artifact: &Artifact,
    field: &ArgentFieldArtifact,
    value: &Value,
    values: &ValueEnvironment<'_>,
    context: &str,
) -> Result<ArtifactValue, String> {
    let Some(source_type) = &field.source_type else {
        return convert_value(artifact, &field.ty, value, values, context);
    };
    if let Some(state) = &source_type.actor_state {
        let actor_handle = |value: &Value, context: &str| {
            let actor = value.as_str().ok_or_else(|| format!("{context}: expected an authored actor path"))?;
            let actor = parse_actor_path(&Value::String(actor.to_string()), context).map_err(|error| error.to_string())?;
            values
                .builder
                .actor_type_handle(actor, state)
                .map(ArtifactValue::Bytes)
                .map_err(|error| format!("{context}: failed to resolve actor_type<{state}>: {error}"))
        };
        return match source_type.array {
            None => actor_handle(value, context),
            Some(SourceArrayArtifact::Dynamic) => value
                .as_array()
                .ok_or_else(|| format!("{context}: expected an array of actor paths"))?
                .iter()
                .enumerate()
                .map(|(index, value)| actor_handle(value, &format!("{context}[{index}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(ArtifactValue::Array),
            Some(SourceArrayArtifact::Fixed(len)) => {
                let array = value.as_array().ok_or_else(|| format!("{context}: expected {len} actor paths"))?;
                if array.len() != len {
                    return Err(format!("{context}: expected {len} actor paths, got {}", array.len()));
                }
                array
                    .iter()
                    .enumerate()
                    .map(|(index, value)| actor_handle(value, &format!("{context}[{index}]")))
                    .collect::<Result<Vec<_>, _>>()
                    .map(ArtifactValue::Array)
            }
        };
    }
    convert_value(artifact, &field.ty, value, values, context)
}

fn convert_entry_args(
    artifact: &Artifact,
    actor: &ActorArtifact,
    entry: &EntryArtifact,
    contract: &SilContractArtifact,
    source: &[Value],
    values: &ValueEnvironment<'_>,
    context: &str,
) -> Result<Vec<TestArg>, String> {
    let sil_entry = contract.entry(&entry.abi.entry).ok_or_else(|| {
        format!("{context}: entry `{}::{}` references missing Sil ABI entry `{}`", actor.name, entry.name, entry.abi.entry)
    })?;
    let authored_count = sil_entry.params.len().checked_sub(entry.hidden_params.len()).ok_or_else(|| {
        format!(
            "{context}: artifact has {} generated parameters but only {} total ABI parameters",
            entry.hidden_params.len(),
            sil_entry.params.len()
        )
    })?;
    if source.len() != authored_count {
        return Err(format!(
            "{context}: `{}::{}` expects {authored_count} authored arguments, got {}",
            actor.name,
            entry.name,
            source.len()
        ));
    }

    source
        .iter()
        .zip(&sil_entry.params[..authored_count])
        .enumerate()
        .map(|(index, (value, param))| {
            let arg_context = format!("{context}[{index}] ({})", param.name);
            if let Some(name) = directive(value, "sign", &arg_context)? {
                if !matches!(param.ty, TypeArtifact::Sig) {
                    return Err(format!("{arg_context}: `sign` is valid only for a `sig` argument"));
                }
                return named_key(values.keys, name, &arg_context).cloned().map(TestArg::Sign);
            }
            if let Some(selector) = entry.template_selectors.iter().find(|selector| selector.name == param.name) {
                let selected = value
                    .as_str()
                    .ok_or_else(|| format!("{arg_context}: actor argument `{}` must be an authored actor name", param.name))?;
                if selector.variants.iter().all(|variant| variant != selected) {
                    return Err(format!(
                        "{arg_context}: actor `{selected}` is not one of `{}`'s variants ({})",
                        selector.actor_enum,
                        selector.variants.join(", ")
                    ));
                }
                Ok(TestArg::Static(ArgValue::Actor(selected.to_string())))
            } else {
                convert_value(artifact, &param.ty, value, values, &arg_context).map(ArgValue::Value).map(TestArg::Static)
            }
        })
        .collect()
}

fn convert_value(
    artifact: &Artifact,
    ty: &TypeArtifact,
    value: &Value,
    values: &ValueEnvironment<'_>,
    context: &str,
) -> Result<ArtifactValue, String> {
    if let Some(name) = directive(value, "pubkey", context)? {
        if !matches!(ty, TypeArtifact::Pubkey) {
            return Err(format!("{context}: `pubkey` is valid only for a `pubkey` value"));
        }
        return key_pubkey(named_key(values.keys, name, context)?).map(ArtifactValue::Bytes);
    }
    if let Some(name) = directive(value, "key_hash", context)? {
        if !matches!(ty, TypeArtifact::FixedBytes { len: 32 } | TypeArtifact::Bytes) {
            return Err(format!("{context}: `key_hash` requires a 32-byte or byte-array value"));
        }
        let public_key = key_pubkey(named_key(values.keys, name, context)?)?;
        return Ok(ArtifactValue::Bytes(blake2b32(&public_key).to_vec()));
    }
    if let Some(label) = directive(value, "covenant", context)? {
        if !matches!(ty, TypeArtifact::FixedBytes { len: 32 } | TypeArtifact::Bytes) {
            return Err(format!("{context}: `covenant` requires a 32-byte or byte-array value"));
        }
        return values
            .covenant_ids
            .get(label)
            .copied()
            .map(|id| ArtifactValue::Bytes(id.as_bytes().to_vec()))
            .ok_or_else(|| format!("{context}: unknown covenant label `{label}`"));
    }
    if directive(value, "sign", context)?.is_some() {
        return Err(format!("{context}: `sign` is valid only as a top-level authored `sig` argument"));
    }

    match ty {
        TypeArtifact::Int => value.as_i64().map(ArtifactValue::Int).ok_or_else(|| format!("{context}: expected an integer")),
        TypeArtifact::Bool => value.as_bool().map(ArtifactValue::Bool).ok_or_else(|| format!("{context}: expected a boolean")),
        TypeArtifact::Byte => value
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .map(ArtifactValue::Byte)
            .ok_or_else(|| format!("{context}: expected an integer from 0 through 255")),
        TypeArtifact::Bytes => decode_hex(value, context, None).map(ArtifactValue::Bytes),
        TypeArtifact::Text => {
            value.as_str().map(|value| ArtifactValue::Text(value.to_string())).ok_or_else(|| format!("{context}: expected a string"))
        }
        TypeArtifact::Pubkey => decode_hex(value, context, Some(32)).map(ArtifactValue::Bytes),
        TypeArtifact::Sig => decode_hex(value, context, Some(65)).map(ArtifactValue::Bytes),
        TypeArtifact::Datasig => decode_hex(value, context, Some(64)).map(ArtifactValue::Bytes),
        TypeArtifact::FixedBytes { len } => decode_hex(value, context, Some(*len)).map(ArtifactValue::Bytes),
        TypeArtifact::FixedArray { item, len } => {
            let array = value.as_array().ok_or_else(|| format!("{context}: expected an array of length {len}"))?;
            if array.len() != *len {
                return Err(format!("{context}: expected an array of length {len}, got {}", array.len()));
            }
            array
                .iter()
                .enumerate()
                .map(|(index, value)| convert_value(artifact, item, value, values, &format!("{context}[{index}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(ArtifactValue::Array)
        }
        TypeArtifact::DynamicArray { item } => value
            .as_array()
            .ok_or_else(|| format!("{context}: expected an array"))?
            .iter()
            .enumerate()
            .map(|(index, value)| convert_value(artifact, item, value, values, &format!("{context}[{index}]")))
            .collect::<Result<Vec<_>, _>>()
            .map(ArtifactValue::Array),
        TypeArtifact::Struct { name } => {
            if artifact.argent.states.iter().any(|state| state.name == *name) {
                let source = value.as_object().ok_or_else(|| format!("{context}: expected an object of type `{name}`"))?;
                return convert_argent_state(artifact, name, source, values, context).map(ArtifactValue::Object);
            }
            let state = artifact
                .sil_abi
                .states
                .iter()
                .find(|state| state.name == *name)
                .ok_or_else(|| format!("{context}: artifact has no struct state `{name}`"))?;
            let source = value.as_object().ok_or_else(|| format!("{context}: expected an object of type `{name}`"))?;
            convert_object(artifact, state.fields.iter().map(|field| (field.name.as_str(), &field.ty)), source, values, context)
                .map(ArtifactValue::Object)
        }
    }
}

fn convert_object<'a>(
    artifact: &Artifact,
    fields: impl Iterator<Item = (&'a str, &'a TypeArtifact)>,
    source: &Map<String, Value>,
    values: &ValueEnvironment<'_>,
    context: &str,
) -> Result<BTreeMap<String, ArtifactValue>, String> {
    let fields = fields.collect::<Vec<_>>();
    if let Some(extra) = source.keys().find(|name| fields.iter().all(|(field, _)| field != &name.as_str())) {
        return Err(format!("{context}: unknown field `{extra}`"));
    }
    let mut converted = BTreeMap::new();
    for (name, ty) in fields {
        let value = source.get(name).ok_or_else(|| format!("{context}: missing field `{name}`"))?;
        converted.insert(name.to_string(), convert_value(artifact, ty, value, values, &format!("{context}.{name}"))?);
    }
    Ok(converted)
}

fn directive<'a>(value: &'a Value, directive: &str, context: &str) -> Result<Option<&'a str>, String> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if !object.contains_key(directive) {
        return Ok(None);
    }
    if object.len() != 1 {
        return Err(format!("{context}: `{directive}` directive cannot contain other fields"));
    }
    object[directive].as_str().map(Some).ok_or_else(|| format!("{context}: `{directive}` must name a test key or covenant"))
}

fn named_key<'a>(keys: &'a BTreeMap<String, TestKey>, name: &str, context: &str) -> Result<&'a TestKey, String> {
    keys.get(name).ok_or_else(|| format!("{context}: unknown test key `{name}`"))
}

fn key_pubkey(key: &TestKey) -> Result<Vec<u8>, String> {
    let secret = SecretKey::from_slice(&key.0).map_err(|error| format!("invalid test secret: {error}"))?;
    let pair = Keypair::from_secret_key(&Secp256k1::new(), &secret);
    Ok(pair.x_only_public_key().0.serialize().to_vec())
}

fn entry_call(name: String, args: Vec<TestArg>) -> EntryCall<'static> {
    if args.iter().all(|arg| matches!(arg, TestArg::Static(_))) {
        return EntryCall::new(name).args(
            args.into_iter()
                .map(|arg| match arg {
                    TestArg::Static(value) => value,
                    TestArg::Sign(_) => unreachable!("checked static arguments"),
                })
                .collect(),
        );
    }
    EntryCall::new(name).try_args_with(move |transaction, input_index| {
        args.iter()
            .map(|arg| match arg {
                TestArg::Static(value) => Ok(value.clone()),
                TestArg::Sign(key) => {
                    sign_input(transaction, input_index, key).map(|signature| ArgValue::Value(ArtifactValue::Bytes(signature)))
                }
            })
            .collect::<Result<Vec<_>, SigningError>>()
    })
}

fn sign_input(transaction: &MutableTransaction<Transaction>, input_index: usize, key: &TestKey) -> Result<Vec<u8>, SigningError> {
    let secret = SecretKey::from_slice(&key.0).map_err(|error| SigningError(format!("invalid test secret: {error}")))?;
    let pair = Keypair::from_secret_key(&Secp256k1::new(), &secret);
    let reused = SigHashReusedValuesUnsync::new();
    let digest = calc_schnorr_signature_hash(&transaction.as_verifiable(), input_index, SIG_HASH_ALL, &reused);
    let message = Message::from_digest_slice(digest.as_bytes().as_slice())
        .map_err(|error| SigningError(format!("invalid signature digest: {error}")))?;
    let mut signature = pair.sign_schnorr(message).as_ref().to_vec();
    signature.push(SIG_HASH_ALL.to_u8());
    Ok(signature)
}

fn derive_output_authorizers(
    runner: &ArgentTestRunner,
    case: &str,
    inputs: &[ResolvedInput],
    outputs: &[TestOutput],
) -> Result<Vec<usize>, CaseError> {
    let expected = inputs.iter().map(|input| input.route_outputs.len()).sum::<usize>();
    if outputs.len() != expected {
        return Err(CaseError::in_case(
            case,
            format!("transaction shape declares {expected} routed actor outputs, but the sidecar provides {}", outputs.len()),
        ));
    }
    let mut next_route = vec![0usize; inputs.len()];
    let mut authorizers = Vec::with_capacity(outputs.len());
    for (output_index, output) in outputs.iter().enumerate() {
        let candidates = inputs
            .iter()
            .enumerate()
            .filter_map(|(input_index, input)| {
                input
                    .route_outputs
                    .get(next_route[input_index])
                    .is_some_and(|actors| actors.iter().any(|actor| runner.actor_paths_equivalent(actor, &output.actor)))
                    .then_some(input_index)
            })
            .collect::<Vec<_>>();
        let [authorizer] = candidates.as_slice() else {
            let detail = if candidates.is_empty() { "does not match any next routed output" } else { "is ambiguous between inputs" };
            return Err(CaseError::in_case(case, format!("output {output_index} actor `{}` {detail}", output.actor)));
        };
        next_route[*authorizer] += 1;
        authorizers.push(*authorizer);
    }
    if let Some((input, _)) = inputs.iter().zip(&next_route).find(|(input, used)| **used != input.route_outputs.len()) {
        return Err(CaseError::in_case(case, format!("entry `{}::{}` has unmatched routed outputs", input.actor, input.entry)));
    }
    Ok(authorizers)
}

fn route_actor_path(owner: &ActorPath, route_actor: &str) -> ActorPath {
    let route_actor = ActorPath::from(route_actor);
    match route_actor.app {
        Some(app) => ActorPath::qualified(super::app_alias(&app), route_actor.actor),
        None => match &owner.app {
            Some(app) => ActorPath::qualified(app.clone(), route_actor.actor),
            None => ActorPath::primary(route_actor.actor),
        },
    }
}

fn decode_hex(value: &Value, context: &str, expected_len: Option<usize>) -> Result<Vec<u8>, String> {
    let value = value.as_str().ok_or_else(|| format!("{context}: expected a `0x`-prefixed hexadecimal string"))?;
    let raw = value.strip_prefix("0x").ok_or_else(|| format!("{context}: expected a `0x`-prefixed hexadecimal string"))?;
    if raw.len() % 2 != 0 {
        return Err(format!("{context}: hexadecimal data must contain an even number of digits"));
    }
    let bytes = crate::codec::decode_hex(raw).map_err(|error| format!("{context}: invalid hexadecimal data: {error}"))?;
    if let Some(expected) = expected_len
        && bytes.len() != expected
    {
        return Err(format!("{context}: expected {expected} bytes, got {}", bytes.len()));
    }
    Ok(bytes)
}

fn derive_hash(case: &str, kind: &str, label: &str) -> Hash {
    let digest = Blake2bParams::new()
        .hash_length(32)
        .to_state()
        .update(TEST_HASH_DOMAIN)
        .update(&[0])
        .update(case.as_bytes())
        .update(&[0])
        .update(kind.as_bytes())
        .update(&[0])
        .update(label.as_bytes())
        .finalize();
    Hash::from_bytes(digest.as_bytes().try_into().expect("BLAKE2b test ids are exactly 32 bytes"))
}

fn blake2b32(value: &[u8]) -> [u8; 32] {
    Blake2bParams::new()
        .hash_length(32)
        .to_state()
        .update(value)
        .finalize()
        .as_bytes()
        .try_into()
        .expect("BLAKE2b hashes are exactly 32 bytes")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_defaults_keys_and_stable_rejection_target() {
        let file = parse_test_file(
            json!({
                "keys":{"owner":format!("0x{}", "01".repeat(32))},
                "tests":[{
                    "name":"reject stale ticket",
                    "inputs":[{"actor":"Ticket", "state":{"serial":1}, "entry":"redeem"}],
                    "outputs":[],
                    "expect":{"reject":"Ticket::redeem"}
                }]
            }),
            "inline.test.json",
        )
        .expect("sidecar parses");

        assert_eq!(file.tests[0].keys.len(), 1);
        assert_eq!(file.tests[0].inputs[0].value, DEFAULT_ACTOR_VALUE);
        assert_eq!(file.tests[0].expect, TestExpectation::Reject(Some("Ticket::redeem".to_string())));
    }

    #[test]
    fn keeps_shared_covenant_labels_high_level() {
        let file = parse_test_file(
            json!({
                "tests":[{
                    "name":"pair",
                    "inputs":[
                        {"actor":"Left", "state":{}, "entry":"shift", "args":[3], "covenant":"pair"},
                        {"actor":"Right", "state":{}, "entry":"accept", "covenant":"pair"}
                    ],
                    "outputs":[],
                    "expect":"accept"
                }]
            }),
            "inline.test.json",
        )
        .expect("sidecar parses");

        assert_eq!(file.tests[0].inputs[0].covenant.as_deref(), Some("pair"));
        assert_eq!(file.tests[0].inputs[1].covenant.as_deref(), Some("pair"));
    }

    #[test]
    fn rejects_raw_transaction_fields() {
        let error = parse_test_file(
            json!({
                "tests":[{
                    "name":"raw",
                    "inputs":[{"actor":"Ticket", "state":{}, "entry":"redeem", "outpoint":"00"}],
                    "outputs":[],
                    "expect":"reject"
                }]
            }),
            "inline.test.json",
        )
        .expect_err("raw fields are outside the Argent schema");

        assert!(error.to_string().contains("unknown field `outpoint`"), "{error}");
    }

    #[test]
    fn route_actor_paths_keep_local_and_foreign_app_identity() {
        let owner = ActorPath::qualified("owner_app", "Source");

        assert_eq!(route_actor_path(&owner, "Cell"), ActorPath::qualified("owner_app", "Cell"));
        assert_eq!(route_actor_path(&owner, "TargetApp::Cell"), ActorPath::qualified("target_app", "Cell"));
        assert_ne!(route_actor_path(&owner, "Cell"), route_actor_path(&owner, "TargetApp::Cell"));
    }

    #[test]
    fn source_app_names_are_normalized_in_paths_and_targets() {
        let actor = parse_actor_path(&json!("TargetApp::Cell"), "actor").expect("qualified actor parses");
        let expectation = parse_expectation(&json!({ "reject": "TargetApp::Cell::go" }), "expect").expect("target parses");

        assert_eq!(actor, ActorPath::qualified("target_app", "Cell"));
        assert_eq!(expectation, TestExpectation::Reject(Some("target_app::Cell::go".to_string())));
    }
}
