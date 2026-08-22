//! Argent-level transaction expectations and generated-source failure replay.
//!
//! The runtime exposes script rejection as data. This module adds the compiler,
//! filesystem, test-schema, and presentation concerns that turn that data into
//! an Argent test experience.

mod case;
mod suite;

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use debugger_session::{
    format_failure_report, format_value,
    session::{DebugEngine, DebugSession, ShadowTxContext},
};
use kaspa_consensus_core::{
    hashing::sighash::SigHashReusedValuesUnsync,
    tx::{PopulatedTransaction, Transaction, VerifiableTransaction},
};
use kaspa_txscript::{
    EngineCtx, caches::Cache, covenants::CovenantsContext, pay_to_script_hash_script, pay_to_script_hash_signature_script_with_flags,
};
use silverscript_lang::{
    ast::{ArrayDim, Expr, TypeBase, TypeRef},
    compiler::{CompileOptions, compile_contract},
};
use thiserror::Error;

use crate::{
    artifact::{Artifact, ArtifactValue, TypeArtifact, decode_runtime_state_script},
    builder::{
        ActorPath, ArtifactBundle, BuilderError, BuilderResult, FailedScript, ScriptFailure, TxBuilder, TxContext, TxOutcome,
        covenant_engine_flags,
    },
};

pub use case::{TestCase, TestExpectation, TestFile};
pub use suite::{TestCaseReport, TestStatus, TestSuiteReport, render_test_report};

pub type TestResult<T> = std::result::Result<T, ArgentTestError>;

#[derive(Debug)]
pub(super) struct LoadedApp {
    pub(super) artifact: Artifact,
    pub(super) dir: PathBuf,
}

/// Loads compiled Argent artifacts and evaluates transaction expectations.
#[derive(Debug)]
pub struct ArgentTestRunner {
    build_dir: PathBuf,
    primary_app: String,
    apps: BTreeMap<String, LoadedApp>,
}

impl ArgentTestRunner {
    /// Load a primary artifact and all compiled app dependencies from a build directory.
    pub fn from_build_dir(build_dir: impl AsRef<Path>) -> TestResult<Self> {
        let requested_dir = build_dir.as_ref();
        let build_dir = fs::canonicalize(requested_dir).map_err(|error| {
            ArgentTestError::Artifact(format!("failed to resolve build directory `{}`: {error}", requested_dir.display()))
        })?;
        if !build_dir.is_dir() {
            return Err(ArgentTestError::Artifact(format!("build path `{}` is not a directory", build_dir.display())));
        }
        let primary = load_app(&build_dir)?;
        let primary_app = app_alias(&primary.artifact.app);
        let mut dependencies = VecDeque::from(primary.artifact.dependencies.clone());
        let mut apps = BTreeMap::from([(primary_app.clone(), primary)]);

        while let Some(dependency) = dependencies.pop_front() {
            let alias = app_alias(&dependency.app);
            if let Some(loaded) = apps.get(&alias) {
                if loaded.artifact.app != dependency.app || loaded.artifact.id != dependency.artifact_id {
                    return Err(ArgentTestError::Artifact(format!(
                        "dependency `{}` expects artifact `{}`, found app `{}` artifact `{}`",
                        dependency.app, dependency.artifact_id, loaded.artifact.app, loaded.artifact.id
                    )));
                }
                continue;
            }

            let loaded = load_app(&dependency_app_dir(&build_dir, &dependency.app)?)?;
            if loaded.artifact.app != dependency.app || loaded.artifact.id != dependency.artifact_id {
                return Err(ArgentTestError::Artifact(format!(
                    "dependency `{}` expects artifact `{}`, found app `{}` artifact `{}`",
                    dependency.app, dependency.artifact_id, loaded.artifact.app, loaded.artifact.id
                )));
            }
            dependencies.extend(loaded.artifact.dependencies.iter().cloned());
            apps.insert(alias, loaded);
        }

        Ok(Self { build_dir, primary_app, apps })
    }

    /// Build the lightweight runtime facade for the loaded app bundle.
    pub fn builder(&self) -> BuilderResult<TxBuilder<'_>> {
        let primary = &self.apps[&self.primary_app].artifact;
        let mut bundle = ArtifactBundle::new(primary)?;
        for (alias, loaded) in &self.apps {
            if alias != &self.primary_app {
                bundle = bundle.with_artifact(&loaded.artifact)?;
            }
        }
        TxBuilder::from_bundle(&bundle)
    }

    /// Assert that transaction construction and script evaluation both accept.
    pub fn expect_accept(&self, name: impl Into<String>, builder: &TxBuilder<'_>, context: &TxContext<'_>) -> TestResult<Transaction> {
        let name = name.into();
        match builder.evaluate(context).map_err(|source| ArgentTestError::Setup { name: name.clone(), source: Box::new(source) })? {
            TxOutcome::Accepted(transaction) => Ok(transaction),
            TxOutcome::Rejected(failure) => {
                let target = failure_target(&failure);
                let reason = failure.source.to_string();
                let report = self.failure_explanation(&failure);
                Err(ArgentTestError::UnexpectedRejection { name, target, reason, report, failure: Box::new(failure) })
            }
        }
    }

    /// Assert that script evaluation rejects, returning its exact replay material.
    pub fn expect_reject(
        &self,
        name: impl Into<String>,
        builder: &TxBuilder<'_>,
        context: &TxContext<'_>,
    ) -> TestResult<ScriptFailure> {
        let name = name.into();
        match builder.evaluate(context).map_err(|source| ArgentTestError::Setup { name: name.clone(), source: Box::new(source) })? {
            TxOutcome::Rejected(failure) => Ok(failure),
            TxOutcome::Accepted(transaction) => {
                Err(ArgentTestError::UnexpectedAcceptance { name, transaction: Box::new(transaction) })
            }
        }
    }

    /// Assert that a particular authored actor entry rejects.
    pub fn expect_reject_at(
        &self,
        name: impl Into<String>,
        target: impl AsRef<str>,
        builder: &TxBuilder<'_>,
        context: &TxContext<'_>,
    ) -> TestResult<ScriptFailure> {
        let name = name.into();
        let expected = target.as_ref().to_string();
        let failure = self.expect_reject(name.clone(), builder, context)?;
        let actual = actor_failure_target(&failure);
        if rejection_target_matches(self, &failure, &expected) {
            return Ok(failure);
        }

        let reason = failure.source.to_string();
        let report = self.failure_explanation(&failure);
        Err(ArgentTestError::WrongRejectionTarget {
            name,
            expected,
            actual: actual.unwrap_or_else(|| format!("ordinary input {}", failure.input_index)),
            reason,
            report,
            failure: Box::new(failure),
        })
    }

    pub fn build_dir(&self) -> &Path {
        &self.build_dir
    }

    pub fn primary_artifact(&self) -> &Artifact {
        &self.apps[&self.primary_app].artifact
    }

    pub(super) fn loaded_app_for_actor(&self, actor: &ActorPath) -> Option<&LoadedApp> {
        actor.app.as_deref().map_or_else(|| self.apps.get(&self.primary_app), |app| self.apps.get(app))
    }

    pub(super) fn actor_paths_equivalent(&self, left: &ActorPath, right: &ActorPath) -> bool {
        left.actor == right.actor
            && self
                .loaded_app_for_actor(left)
                .zip(self.loaded_app_for_actor(right))
                .is_some_and(|(left, right)| left.artifact.id == right.artifact.id)
    }

    fn failure_explanation(&self, failure: &ScriptFailure) -> Option<Box<str>> {
        if !matches!(&failure.script, FailedScript::Actor { .. }) {
            return None;
        }
        Some(match self.replay_failure(failure) {
            Ok(report) => report.into_boxed_str(),
            Err(error) => format!("Silver explanation unavailable: {error}").into_boxed_str(),
        })
    }

    fn replay_failure(&self, failure: &ScriptFailure) -> std::result::Result<String, ReplayError> {
        let FailedScript::Actor { actor, entry: _, action_script, redeem_script } = &failure.script else {
            return Err(ReplayError::new("ordinary input scripts have no generated Silver source"));
        };
        let loaded =
            self.loaded_app_for_actor(actor).ok_or_else(|| ReplayError::new(format!("no loaded artifact for actor `{actor}`")))?;
        let contract = loaded
            .artifact
            .sil_abi
            .contract(&actor.actor)
            .ok_or_else(|| ReplayError::new(format!("artifact has no actor `{actor}`")))?;
        let source_path = generated_source_path(&loaded.dir, &contract.source_path)?;
        let source = fs::read_to_string(&source_path)
            .map_err(|error| ReplayError::new(format!("failed to read `{}`: {error}", source_path.display())))?;

        let (_, state_script, _) = contract
            .compiled
            .script_parts(redeem_script)
            .ok_or_else(|| ReplayError::new(format!("executed redeem script does not fit `{actor}` state layout")))?;
        let state = decode_runtime_state_script(&contract.runtime_state, state_script)
            .map_err(|error| ReplayError::new(format!("failed to decode `{actor}` runtime state: {error}")))?;
        let constructor_args = contract
            .runtime_state
            .fields
            .iter()
            .map(|field| {
                let value =
                    state.get(&field.name).ok_or_else(|| ReplayError::new(format!("decoded state has no field `{}`", field.name)))?;
                artifact_constructor_expr(value, &field.ty)
                    .map_err(|error| ReplayError::new(format!("failed to materialize state field `{}`: {error}", field.name)))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let compiled = compile_contract(&source, &constructor_args, CompileOptions { record_debug_infos: true, ..Default::default() })
            .map_err(|error| ReplayError::new(format!("failed to compile `{}` for debugging: {error}", source_path.display())))?;
        if compiled.bytecode != *redeem_script {
            return Err(ReplayError::new(format!("debug bytecode for `{actor}` does not match the executed redeem script")));
        }

        if failure.transaction.inputs.len() != failure.entries.len() {
            return Err(ReplayError::new(format!(
                "transaction has {} inputs but replay material has {} UTXO entries",
                failure.transaction.inputs.len(),
                failure.entries.len()
            )));
        }
        let input = failure
            .transaction
            .inputs
            .get(failure.input_index)
            .ok_or_else(|| ReplayError::new(format!("transaction has no input {}", failure.input_index)))?;
        let expected_signature_script =
            pay_to_script_hash_signature_script_with_flags(redeem_script.clone(), action_script.clone(), covenant_engine_flags())
                .map_err(|error| ReplayError::new(format!("failed to restore the actor signature script: {error}")))?;
        if input.signature_script != expected_signature_script {
            return Err(ReplayError::new("actor replay scripts do not match the rejected transaction input"));
        }

        let populated = PopulatedTransaction::new(&failure.transaction, failure.entries.clone());
        let utxo = populated
            .utxo(failure.input_index)
            .ok_or_else(|| ReplayError::new(format!("replay has no UTXO for input {}", failure.input_index)))?;
        if utxo.script_public_key != pay_to_script_hash_script(redeem_script) {
            return Err(ReplayError::new("executed redeem script does not match the rejected input's UTXO"));
        }
        let covenants = CovenantsContext::from_tx(&populated)
            .map_err(|error| ReplayError::new(format!("failed to restore covenant context: {error}")))?;
        let sig_cache = Cache::new(100);
        let reused = SigHashReusedValuesUnsync::new();
        let engine_context = EngineCtx::new(&sig_cache).with_reused(&reused).with_covenants_ctx(&covenants);
        let engine =
            DebugEngine::from_transaction_input(&populated, input, failure.input_index, utxo, engine_context, covenant_engine_flags());
        let shadow =
            ShadowTxContext { tx: &populated, input, input_index: failure.input_index, utxo_entry: utxo, covenants_ctx: &covenants };
        let mut session = DebugSession::full(action_script, redeem_script, &source, compiled.debug_info, engine)
            .map_err(|error| ReplayError::new(format!("failed to start Silver replay: {error}")))?
            .with_shadow_tx_context(shadow);

        match session.run_to_completion() {
            Ok(()) => Err(ReplayError::new("debug replay accepted a transaction rejected by the runtime")),
            Err(error) => {
                if error != failure.source {
                    return Err(ReplayError::new(format!(
                        "debug replay rejected differently: runtime reported `{}`, replay reported `{error}`",
                        failure.source
                    )));
                }
                let report = session.build_failure_report(&error);
                Ok(format_failure_report(&report, &format_value))
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum ArgentTestError {
    #[error("Argent test artifact error: {0}")]
    Artifact(String),
    #[error("Argent test definition error: {0}")]
    Definition(String),
    #[error("test `{name}` error: {source}")]
    Setup {
        name: String,
        #[source]
        source: Box<BuilderError>,
    },
    #[error("test `{name}` expected accept, but {target} rejected: {reason}{report_suffix}", report_suffix = optional_report(.report))]
    UnexpectedRejection { name: String, target: String, reason: String, report: Option<Box<str>>, failure: Box<ScriptFailure> },
    #[error("test `{name}` expected reject, but the transaction was accepted")]
    UnexpectedAcceptance { name: String, transaction: Box<Transaction> },
    #[error("test `{name}` expected rejection from `{expected}`, but `{actual}` rejected: {reason}{report_suffix}", report_suffix = optional_report(.report))]
    WrongRejectionTarget {
        name: String,
        expected: String,
        actual: String,
        reason: String,
        report: Option<Box<str>>,
        failure: Box<ScriptFailure>,
    },
}

impl ArgentTestError {
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::UnexpectedRejection { .. } | Self::UnexpectedAcceptance { .. } | Self::WrongRejectionTarget { .. })
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
struct ReplayError {
    message: String,
}

impl ReplayError {
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

fn load_app(dir: &Path) -> TestResult<LoadedApp> {
    let dir = fs::canonicalize(dir)
        .map_err(|error| ArgentTestError::Artifact(format!("failed to resolve artifact directory `{}`: {error}", dir.display())))?;
    let artifact_path = dir.join("artifact.json");
    let json = fs::read_to_string(&artifact_path)
        .map_err(|error| ArgentTestError::Artifact(format!("failed to read `{}`: {error}", artifact_path.display())))?;
    let artifact: Artifact = serde_json::from_str(&json)
        .map_err(|error| ArgentTestError::Artifact(format!("failed to parse `{}`: {error}", artifact_path.display())))?;
    artifact
        .check_schema_version()
        .map_err(|error| ArgentTestError::Artifact(format!("invalid `{}`: {error}", artifact_path.display())))?;
    artifact.verify_sil_abi().map_err(|error| ArgentTestError::Artifact(format!("invalid `{}`: {error}", artifact_path.display())))?;
    artifact
        .verify_template_plan()
        .map_err(|error| ArgentTestError::Artifact(format!("invalid `{}`: {error}", artifact_path.display())))?;
    artifact.verify_id().map_err(|error| ArgentTestError::Artifact(format!("invalid `{}`: {error}", artifact_path.display())))?;
    for contract in &artifact.sil_abi.contracts {
        relative_generated_source_path(&contract.source_path).map_err(|error| {
            ArgentTestError::Artifact(format!(
                "invalid generated source for `{}` in `{}`: {error}",
                contract.name,
                artifact_path.display()
            ))
        })?;
    }
    Ok(LoadedApp { artifact, dir })
}

fn dependency_app_dir(build_dir: &Path, app: &str) -> TestResult<PathBuf> {
    use std::path::Component;

    let mut components = Path::new(app).components();
    let Some(Component::Normal(app_dir)) = components.next() else {
        return Err(ArgentTestError::Artifact(format!("dependency app `{app}` is not a single path component")));
    };
    if components.next().is_some() {
        return Err(ArgentTestError::Artifact(format!("dependency app `{app}` is not a single path component")));
    }
    let apps_dir = fs::canonicalize(build_dir.join("apps")).map_err(|error| {
        ArgentTestError::Artifact(format!("failed to resolve dependency directory below `{}`: {error}", build_dir.display()))
    })?;
    if !apps_dir.starts_with(build_dir) || !apps_dir.is_dir() {
        return Err(ArgentTestError::Artifact(format!(
            "dependency directory `{}` escapes build directory `{}`",
            apps_dir.display(),
            build_dir.display()
        )));
    }
    let app_dir = fs::canonicalize(apps_dir.join(app_dir))
        .map_err(|error| ArgentTestError::Artifact(format!("failed to resolve artifact directory for dependency `{app}`: {error}")))?;
    if !app_dir.starts_with(&apps_dir) || !app_dir.is_dir() {
        return Err(ArgentTestError::Artifact(format!(
            "dependency artifact directory `{}` escapes `{}`",
            app_dir.display(),
            apps_dir.display()
        )));
    }
    Ok(app_dir)
}

fn generated_source_path(dir: &Path, source: &str) -> std::result::Result<PathBuf, ReplayError> {
    let relative = relative_generated_source_path(source)?;
    let artifact_dir = fs::canonicalize(dir)
        .map_err(|error| ReplayError::new(format!("failed to resolve artifact directory `{}`: {error}", dir.display())))?;
    let source_path = fs::canonicalize(artifact_dir.join(relative))
        .map_err(|error| ReplayError::new(format!("failed to resolve generated source `{source}`: {error}")))?;
    if !source_path.starts_with(&artifact_dir) {
        return Err(ReplayError::new(format!(
            "generated source `{}` escapes artifact directory `{}`",
            source_path.display(),
            artifact_dir.display()
        )));
    }
    if !source_path.is_file() {
        return Err(ReplayError::new(format!("generated source `{}` is not a file", source_path.display())));
    }
    Ok(source_path)
}

fn relative_generated_source_path(source: &str) -> std::result::Result<PathBuf, ReplayError> {
    use std::path::Component;

    let mut relative = PathBuf::new();
    for component in Path::new(source).components() {
        match component {
            Component::Normal(component) => relative.push(component),
            _ => return Err(ReplayError::new(format!("generated source path `{source}` is not relative to its artifact"))),
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(ReplayError::new("generated source path is empty"));
    }
    Ok(relative)
}

fn actor_failure_target(failure: &ScriptFailure) -> Option<String> {
    match &failure.script {
        FailedScript::Actor { actor, entry, .. } => Some(format!("{actor}::{entry}")),
        FailedScript::Ordinary => None,
    }
}

fn rejection_target_matches(runner: &ArgentTestRunner, failure: &ScriptFailure, expected: &str) -> bool {
    let Some((actor, expected_entry)) = expected.rsplit_once("::") else {
        return false;
    };
    let mut expected_actor = ActorPath::from(actor);
    if let Some(app) = &mut expected_actor.app {
        *app = app_alias(app);
    }
    match &failure.script {
        FailedScript::Actor { actor, entry, .. } => entry == expected_entry && runner.actor_paths_equivalent(actor, &expected_actor),
        FailedScript::Ordinary => false,
    }
}

fn failure_target(failure: &ScriptFailure) -> String {
    actor_failure_target(failure).unwrap_or_else(|| format!("ordinary input {}", failure.input_index))
}

fn artifact_constructor_expr(value: &ArtifactValue, ty: &TypeArtifact) -> std::result::Result<Expr<'static>, String> {
    match (ty, value) {
        (TypeArtifact::Int, ArtifactValue::Int(value)) => Ok(Expr::int(*value)),
        (TypeArtifact::Bool, ArtifactValue::Bool(value)) => Ok(Expr::bool(*value)),
        (TypeArtifact::Byte, ArtifactValue::Byte(value)) => Ok(Expr::byte(*value)),
        (TypeArtifact::Bytes, ArtifactValue::Bytes(value)) => Ok(Expr::dynamic_bytes(value.clone())),
        (TypeArtifact::Text, ArtifactValue::Text(value)) => Ok(Expr::string(value.clone())),
        (TypeArtifact::Pubkey, ArtifactValue::Bytes(value)) => fixed_bytes_expr("pubkey", value, 32),
        (TypeArtifact::Sig, ArtifactValue::Bytes(value)) => fixed_bytes_expr("sig", value, 65),
        (TypeArtifact::Datasig, ArtifactValue::Bytes(value)) => fixed_bytes_expr("datasig", value, 64),
        (TypeArtifact::FixedBytes { len }, ArtifactValue::Bytes(value)) => fixed_bytes_expr("byte array", value, *len),
        (TypeArtifact::FixedArray { item, len }, ArtifactValue::Array(values)) => {
            if values.len() != *len {
                return Err(format!("array expects {len} elements, got {}", values.len()));
            }
            array_expr(ty, item, values)
        }
        (TypeArtifact::DynamicArray { item }, ArtifactValue::Array(values)) => array_expr(ty, item, values),
        (TypeArtifact::Struct { name }, _) => Err(format!("runtime state struct `{name}` is not supported by the Sil ABI codec")),
        _ => Err(format!("expected {}, got {}", artifact_type_name(ty), artifact_value_name(value))),
    }
}

fn fixed_bytes_expr(name: &str, value: &[u8], expected: usize) -> std::result::Result<Expr<'static>, String> {
    if value.len() != expected {
        return Err(format!("{name} expects {expected} bytes, got {}", value.len()));
    }
    Ok(Expr::bytes(value.to_vec()))
}

fn array_expr(ty: &TypeArtifact, item: &TypeArtifact, values: &[ArtifactValue]) -> std::result::Result<Expr<'static>, String> {
    let values = values.iter().map(|value| artifact_constructor_expr(value, item)).collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(Expr::array(silver_type_ref(ty)?, values))
}

fn silver_type_ref(ty: &TypeArtifact) -> std::result::Result<TypeRef, String> {
    let type_ref = match ty {
        TypeArtifact::Int => TypeRef { base: TypeBase::Int, array_dims: vec![] },
        TypeArtifact::Bool => TypeRef { base: TypeBase::Bool, array_dims: vec![] },
        TypeArtifact::Byte => TypeRef { base: TypeBase::Byte, array_dims: vec![] },
        TypeArtifact::Bytes => TypeRef { base: TypeBase::Byte, array_dims: vec![ArrayDim::Dynamic] },
        TypeArtifact::Text => TypeRef { base: TypeBase::String, array_dims: vec![] },
        TypeArtifact::Pubkey => TypeRef { base: TypeBase::Pubkey, array_dims: vec![] },
        TypeArtifact::Sig => TypeRef { base: TypeBase::Sig, array_dims: vec![] },
        TypeArtifact::Datasig => TypeRef { base: TypeBase::Datasig, array_dims: vec![] },
        TypeArtifact::FixedBytes { len } => TypeRef { base: TypeBase::Byte, array_dims: vec![ArrayDim::Fixed(*len)] },
        TypeArtifact::FixedArray { item, len } => {
            let mut item = silver_type_ref(item)?;
            item.array_dims.push(ArrayDim::Fixed(*len));
            item
        }
        TypeArtifact::DynamicArray { item } => {
            let mut item = silver_type_ref(item)?;
            item.array_dims.push(ArrayDim::Dynamic);
            item
        }
        TypeArtifact::Struct { name } => TypeRef { base: TypeBase::Custom(name.clone()), array_dims: vec![] },
    };
    Ok(type_ref)
}

fn artifact_type_name(ty: &TypeArtifact) -> String {
    match ty {
        TypeArtifact::Int => "int".to_string(),
        TypeArtifact::Bool => "bool".to_string(),
        TypeArtifact::Byte => "byte".to_string(),
        TypeArtifact::Bytes => "bytes".to_string(),
        TypeArtifact::Text => "string".to_string(),
        TypeArtifact::Pubkey => "pubkey".to_string(),
        TypeArtifact::Sig => "sig".to_string(),
        TypeArtifact::Datasig => "datasig".to_string(),
        TypeArtifact::FixedBytes { len } => format!("byte[{len}]"),
        TypeArtifact::FixedArray { item, len } => format!("{}[{len}]", artifact_type_name(item)),
        TypeArtifact::DynamicArray { item } => format!("{}[]", artifact_type_name(item)),
        TypeArtifact::Struct { name } => name.clone(),
    }
}

fn artifact_value_name(value: &ArtifactValue) -> &'static str {
    match value {
        ArtifactValue::Int(_) => "int",
        ArtifactValue::Bool(_) => "bool",
        ArtifactValue::Byte(_) => "byte",
        ArtifactValue::Bytes(_) => "bytes",
        ArtifactValue::Text(_) => "string",
        ArtifactValue::Array(_) => "array",
        ArtifactValue::Object(_) => "object",
    }
}

fn optional_report(report: &Option<Box<str>>) -> String {
    report.as_ref().map(|report| format!("\n\n{report}")).unwrap_or_default()
}

fn app_alias(app: &str) -> String {
    let mut out = String::new();
    let chars = app.chars().collect::<Vec<_>>();
    for (index, ch) in chars.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|previous| chars.get(previous)).copied();
        let next = chars.get(index + 1).copied();
        if ch.is_ascii_uppercase() {
            if index > 0
                && !out.ends_with('_')
                && previous.is_some_and(|previous| {
                    previous.is_ascii_lowercase() || previous.is_ascii_digit() || next.is_some_and(|next| next.is_ascii_lowercase())
                })
            {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            out.push(*ch);
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}
