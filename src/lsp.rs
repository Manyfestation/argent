//! Minimal Language Server Protocol support for Argent editors.
//!
//! The server deliberately stays dependency-free: it speaks JSON-RPC over
//! stdio using `serde_json` and uses a tolerant editor scanner so completion
//! keeps working while a source file is incomplete.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{ArgentError, Result};

const KEYWORDS: &[&str] = &[
    "actor", "app", "as", "become", "by", "consumes", "const", "constant", "delegate", "else", "emits", "entry", "enum", "expands",
    "false", "fn", "for", "from", "if", "import", "inputs", "new", "none", "observes", "outputs", "owns", "return", "self", "spawns",
    "state", "this", "true", "tx", "virtual",
];

const PRIMITIVE_TYPES: &[&str] =
    &["actor_type", "bool", "byte", "bytes", "cov_id", "datasig", "int", "pubkey", "sig", "string", "temporal"];

struct Builtin {
    name: &'static str,
    signature: &'static str,
    params: &'static [&'static str],
    documentation: &'static str,
}

const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "state",
        signature: "state(input_reference) -> AuthoredState",
        params: &["input_reference"],
        documentation: "Reconstruct complete authored state from `self`, a consumed input handle, or an observed input reference.",
    },
    Builtin {
        name: "digest",
        signature: "digest(authored_state) -> byte[32]",
        params: &["authored_state"],
        documentation: "Compute the Blake3 digest of an authored state storage payload.",
    },
    Builtin {
        name: "signed",
        signature: "signed(value: byte) -> int",
        params: &["value"],
        documentation: "Interpret a byte as a signed integer.",
    },
    Builtin {
        name: "unsigned",
        signature: "unsigned(value: byte) -> int",
        params: &["value"],
        documentation: "Interpret a byte as an unsigned integer.",
    },
    Builtin {
        name: "length",
        signature: "length(data: byte[]) -> int",
        params: &["data"],
        documentation: "Return the byte length of `data`.",
    },
    Builtin {
        name: "blake2b",
        signature: "blake2b(data: byte[]) -> byte[32]",
        params: &["data"],
        documentation: "Hash `data` with Blake2b and return 32 bytes.",
    },
    Builtin {
        name: "blake2bWithKey",
        signature: "blake2bWithKey(data: byte[], key: byte[]) -> byte[32]",
        params: &["data", "key"],
        documentation: "Hash `data` with the supplied Blake2b key and return 32 bytes.",
    },
    Builtin {
        name: "blake3",
        signature: "blake3(data: byte[]) -> byte[32]",
        params: &["data"],
        documentation: "Hash `data` with Blake3 and return 32 bytes.",
    },
    Builtin {
        name: "blake3WithKey",
        signature: "blake3WithKey(data: byte[], key: byte[32]) -> byte[32]",
        params: &["data", "key"],
        documentation: "Hash `data` with the supplied 32-byte Blake3 key and return 32 bytes.",
    },
    Builtin {
        name: "sha256",
        signature: "sha256(data: byte[]) -> byte[32]",
        params: &["data"],
        documentation: "Hash `data` with SHA-256 and return 32 bytes.",
    },
    Builtin {
        name: "checkSig",
        signature: "checkSig(signature: sig, public_key: pubkey) -> bool",
        params: &["signature", "public_key"],
        documentation: "Verify a Schnorr transaction signature.",
    },
    Builtin {
        name: "checkSigEcdsa",
        signature: "checkSigEcdsa(signature: sig, public_key: byte[33]) -> bool",
        params: &["signature", "public_key"],
        documentation: "Verify an ECDSA transaction signature.",
    },
    Builtin {
        name: "checkMsgSig",
        signature: "checkMsgSig(signature: datasig, digest: byte[32], public_key: pubkey) -> bool",
        params: &["signature", "digest", "public_key"],
        documentation: "Verify a Schnorr message signature.",
    },
    Builtin {
        name: "checkMsgSigEcdsa",
        signature: "checkMsgSigEcdsa(signature: datasig, digest: byte[32], public_key: byte[33]) -> bool",
        params: &["signature", "digest", "public_key"],
        documentation: "Verify an ECDSA message signature.",
    },
    Builtin {
        name: "co_spent",
        signature: "value.co_spent() -> bool",
        params: &[],
        documentation: "Require the referenced covenant as a valid input in the current transaction.",
    },
    Builtin {
        name: "require",
        signature: "require(condition: bool)",
        params: &["condition"],
        documentation: "Require `condition` to hold for the transition.",
    },
    Builtin {
        name: "unrestricted",
        signature: "unrestricted(output.value)",
        params: &["output.value"],
        documentation: "Mark an output value as unrestricted.",
    },
    Builtin {
        name: "templateHash",
        signature: "templateHash(templatePrefix: byte[], templateSuffix: byte[]) -> byte[32]",
        params: &["templatePrefix", "templateSuffix"],
        documentation: "Hash a redeem script's non-state prefix and suffix while committing to their exact lengths.",
    },
    Builtin {
        name: "ScriptPubKeyP2PK",
        signature: "ScriptPubKeyP2PK(publicKey: pubkey) -> byte[36]",
        params: &["publicKey"],
        documentation: "Construct a pay-to-public-key script public key.",
    },
    Builtin {
        name: "ScriptPubKeyP2SH",
        signature: "ScriptPubKeyP2SH(scriptHash: byte[32]) -> byte[37]",
        params: &["scriptHash"],
        documentation: "Construct a pay-to-script-hash script public key.",
    },
    Builtin {
        name: "ScriptPubKeyP2SHFromRedeemScript",
        signature: "ScriptPubKeyP2SHFromRedeemScript(redeemScript: byte[]) -> byte[37]",
        params: &["redeemScript"],
        documentation: "Construct a pay-to-script-hash script public key from a redeem script.",
    },
    Builtin { name: "OpTxSubnetId", signature: "OpTxSubnetId()", params: &[], documentation: "Kaspa introspection opcode." },
    Builtin { name: "OpTxGas", signature: "OpTxGas()", params: &[], documentation: "Kaspa introspection opcode." },
    Builtin { name: "OpTxLockTime", signature: "OpTxLockTime()", params: &[], documentation: "Kaspa introspection opcode." },
    Builtin { name: "OpTxInputIndex", signature: "OpTxInputIndex()", params: &[], documentation: "Kaspa introspection opcode." },
    Builtin { name: "OpTxPayloadLen", signature: "OpTxPayloadLen()", params: &[], documentation: "Kaspa introspection opcode." },
    Builtin {
        name: "OpTxPayloadSubstr",
        signature: "OpTxPayloadSubstr(start, end)",
        params: &["start", "end"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpOutpointTxId",
        signature: "OpOutpointTxId(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpOutpointIndex",
        signature: "OpOutpointIndex(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxInputScriptSigLen",
        signature: "OpTxInputScriptSigLen(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxInputScriptSigSubstr",
        signature: "OpTxInputScriptSigSubstr(inputIndex, start, end)",
        params: &["inputIndex", "start", "end"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxInputSeq",
        signature: "OpTxInputSeq(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxInputDaaScore",
        signature: "OpTxInputDaaScore(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxInputIsCoinbase",
        signature: "OpTxInputIsCoinbase(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxInputSpkLen",
        signature: "OpTxInputSpkLen(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxInputSpkSubstr",
        signature: "OpTxInputSpkSubstr(inputIndex, start, end)",
        params: &["inputIndex", "start", "end"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxOutputSpkLen",
        signature: "OpTxOutputSpkLen(outputIndex)",
        params: &["outputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpTxOutputSpkSubstr",
        signature: "OpTxOutputSpkSubstr(outputIndex, start, end)",
        params: &["outputIndex", "start", "end"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpAuthOutputCount",
        signature: "OpAuthOutputCount(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpAuthOutputIdx",
        signature: "OpAuthOutputIdx(inputIndex, outputOrdinal)",
        params: &["inputIndex", "outputOrdinal"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpInputCovenantId",
        signature: "OpInputCovenantId(inputIndex)",
        params: &["inputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpOutputCovenantId",
        signature: "OpOutputCovenantId(outputIndex)",
        params: &["outputIndex"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpCovInputCount",
        signature: "OpCovInputCount(covenantId)",
        params: &["covenantId"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpCovInputIdx",
        signature: "OpCovInputIdx(covenantId, inputOrdinal)",
        params: &["covenantId", "inputOrdinal"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpCovOutputCount",
        signature: "OpCovOutputCount(covenantId)",
        params: &["covenantId"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpCovOutputIdx",
        signature: "OpCovOutputIdx(covenantId, outputOrdinal)",
        params: &["covenantId", "outputOrdinal"],
        documentation: "Kaspa introspection opcode.",
    },
    Builtin {
        name: "OpNum2Bin",
        signature: "OpNum2Bin(value, size)",
        params: &["value", "size"],
        documentation: "Kaspa Script numeric conversion opcode.",
    },
    Builtin {
        name: "OpBin2Num",
        signature: "OpBin2Num(data)",
        params: &["data"],
        documentation: "Kaspa Script numeric conversion opcode.",
    },
    Builtin {
        name: "OpChainblockSeqCommit",
        signature: "OpChainblockSeqCommit(blockHash)",
        params: &["blockHash"],
        documentation: "Kaspa chain-block commitment opcode.",
    },
    Builtin {
        name: "g16.verify",
        signature: "g16.verify(verifyingKey: byte[], proof: byte[], publicInput: byte[32], ...)",
        params: &["verifyingKey", "proof", "publicInput"],
        documentation: "Verify a Groth16 proof with one or more public inputs.",
    },
    Builtin {
        name: "r0.g16.verify",
        signature: "r0.g16.verify(journalHash: byte[32], proof: byte[], imageId: byte[32])",
        params: &["journalHash", "proof", "imageId"],
        documentation: "Verify an R0 Groth16 proof.",
    },
    Builtin {
        name: "r0.succinct.verify",
        signature: "r0.succinct.verify(claim, controlIndex, controlDigests, seal, journal, imageId, controlId)",
        params: &["claim", "controlIndex", "controlDigests", "seal", "journal", "imageId", "controlId"],
        documentation: "Verify an R0 succinct proof.",
    },
    Builtin {
        name: "r0.succinct.blake2b.verify",
        signature: "r0.succinct.blake2b.verify(claim, controlIndex, controlDigests, seal, journal, imageId, controlId)",
        params: &["claim", "controlIndex", "controlDigests", "seal", "journal", "imageId", "controlId"],
        documentation: "Verify an R0 succinct proof with a Blake2b claim.",
    },
    Builtin {
        name: "r0.succinct.poseidon2.verify",
        signature: "r0.succinct.poseidon2.verify(claim, controlIndex, controlDigests, seal, journal, imageId, controlId)",
        params: &["claim", "controlIndex", "controlDigests", "seal", "journal", "imageId", "controlId"],
        documentation: "Verify an R0 succinct proof with a Poseidon2 claim.",
    },
    Builtin {
        name: "r0.succinct.sha256.verify",
        signature: "r0.succinct.sha256.verify(claim, controlIndex, controlDigests, seal, journal, imageId, controlId)",
        params: &["claim", "controlIndex", "controlDigests", "seal", "journal", "imageId", "controlId"],
        documentation: "Verify an R0 succinct proof with a SHA-256 claim.",
    },
];

/// Serve LSP requests over stdin/stdout until the client exits.
pub fn serve() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_io(&mut stdin.lock(), &mut stdout.lock())
}

fn serve_io(reader: &mut impl BufRead, writer: &mut impl Write) -> Result<()> {
    let mut server = Server::default();
    while let Some(message) = read_message(reader)? {
        if !server.handle(message, writer)? {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, String>,
    shutdown_requested: bool,
}

impl Server {
    fn handle(&mut self, message: Value, writer: &mut impl Write) -> Result<bool> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(true);
        };
        let id = message.get("id").cloned();
        if self.shutdown_requested && method != "exit" {
            if id.is_some() {
                respond_error(writer, id, -32600, "server has shut down".to_string())?;
            }
            return Ok(true);
        }

        match method {
            "initialize" => {
                respond(
                    writer,
                    id,
                    json!({
                        "capabilities": {
                            "textDocumentSync": 1,
                            "completionProvider": {
                                "resolveProvider": false,
                                "triggerCharacters": ["."]
                            }
                        },
                        "serverInfo": { "name": "argentc", "version": env!("CARGO_PKG_VERSION") }
                    }),
                )?;
            }
            "shutdown" => {
                self.shutdown_requested = true;
                respond(writer, id, Value::Null)?;
            }
            "exit" => return Ok(false),
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    message.pointer("/params/textDocument/uri").and_then(Value::as_str),
                    message.pointer("/params/textDocument/text").and_then(Value::as_str),
                ) {
                    self.documents.insert(uri.to_string(), text.to_string());
                }
            }
            "textDocument/didChange" => {
                let uri = message.pointer("/params/textDocument/uri").and_then(Value::as_str);
                let text = message.pointer("/params/contentChanges/0/text").and_then(Value::as_str);
                if let (Some(uri), Some(text)) = (uri, text) {
                    self.documents.insert(uri.to_string(), text.to_string());
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = message.pointer("/params/textDocument/uri").and_then(Value::as_str) {
                    self.documents.remove(uri);
                }
            }
            "textDocument/completion" => {
                let uri = message.pointer("/params/textDocument/uri").and_then(Value::as_str).unwrap_or_default();
                let source = self.documents.get(uri).cloned().or_else(|| read_uri(uri)).unwrap_or_default();
                let prefix = completion_prefix(&source, message.pointer("/params/position"));
                let items = completion_items(uri, &source, &self.documents, &prefix);
                respond(writer, id, json!({ "isIncomplete": false, "items": items }))?;
            }
            _ if id.is_some() => respond_error(writer, id, -32601, format!("method not found: {method}"))?,
            _ => {}
        }

        Ok(true)
    }
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut content_length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line == "\n" || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            content_length =
                Some(value.trim().parse::<usize>().map_err(|err| ArgentError::new(format!("invalid LSP Content-Length: {err}")))?);
        }
    }

    let length = content_length.ok_or_else(|| ArgentError::new("missing LSP Content-Length header"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map(Some).map_err(|err| ArgentError::new(format!("invalid LSP JSON: {err}")))
}

fn respond(writer: &mut impl Write, id: Option<Value>, result: Value) -> Result<()> {
    let Some(id) = id else {
        return Ok(());
    };
    write_message(writer, &json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn respond_error(writer: &mut impl Write, id: Option<Value>, code: i64, message: String) -> Result<()> {
    let Some(id) = id else {
        return Ok(());
    };
    write_message(writer, &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
}

fn write_message(writer: &mut impl Write, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message).map_err(|err| ArgentError::new(format!("cannot serialize LSP JSON: {err}")))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EditorTokenKind {
    Ident(String),
    String(String),
    Symbol(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorToken {
    kind: EditorTokenKind,
}

impl EditorToken {
    fn ident(&self) -> Option<&str> {
        match &self.kind {
            EditorTokenKind::Ident(value) => Some(value),
            _ => None,
        }
    }

    fn string(&self) -> Option<&str> {
        match &self.kind {
            EditorTokenKind::String(value) => Some(value),
            _ => None,
        }
    }

    fn is_symbol(&self, expected: char) -> bool {
        self.kind == EditorTokenKind::Symbol(expected)
    }
}

fn editor_tokens(source: &str) -> Vec<EditorToken> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        match bytes[pos] {
            byte if byte.is_ascii_whitespace() => pos += 1,
            b'/' if bytes.get(pos + 1) == Some(&b'/') => {
                pos += 2;
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
            }
            b'/' if bytes.get(pos + 1) == Some(&b'*') => {
                pos += 2;
                while pos < bytes.len() && !(bytes[pos] == b'*' && bytes.get(pos + 1) == Some(&b'/')) {
                    pos += 1;
                }
                pos = (pos + 2).min(bytes.len());
            }
            b'"' => {
                pos += 1;
                let mut value = String::new();
                while pos < bytes.len() && bytes[pos] != b'"' {
                    if bytes[pos] == b'\\' && pos + 1 < bytes.len() {
                        pos += 1;
                    }
                    let character = source[pos..].chars().next().expect("position is in source");
                    value.push(character);
                    pos += character.len_utf8();
                }
                pos = (pos + 1).min(bytes.len());
                tokens.push(EditorToken { kind: EditorTokenKind::String(value) });
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = pos;
                pos += 1;
                while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                    pos += 1;
                }
                tokens.push(EditorToken { kind: EditorTokenKind::Ident(source[start..pos].to_string()) });
            }
            byte if byte.is_ascii_digit() => {
                pos += 1;
                while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                    pos += 1;
                }
            }
            byte if byte.is_ascii() => {
                tokens.push(EditorToken { kind: EditorTokenKind::Symbol(byte as char) });
                pos += 1;
            }
            _ => {
                pos += source[pos..].chars().next().expect("position is in source").len_utf8();
            }
        }
    }
    tokens
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScannedSymbol {
    name: String,
    detail: String,
    kind: u8,
    params: Vec<String>,
}

#[derive(Default)]
struct Scan {
    symbols: Vec<ScannedSymbol>,
    imports: Vec<String>,
}

fn scan_document(source: &str) -> Scan {
    let tokens = editor_tokens(source);
    let mut scan = Scan::default();
    let mut depth = 0usize;
    let mut index = 0usize;

    while index < tokens.len() {
        let token = &tokens[index];
        match token.ident() {
            Some("import") if depth == 0 => {
                if let Some(path) = tokens[index + 1..].iter().take_while(|token| !token.is_symbol(';')).find_map(EditorToken::string)
                {
                    scan.imports.push(path.to_string());
                }
            }
            Some("state") if depth == 0 => {
                push_named_symbol(&mut scan.symbols, &tokens, index + 1, "state", 22, &[]);
            }
            Some("actor") if depth == 0 => {
                if tokens.get(index + 1).and_then(EditorToken::ident) == Some("enum") {
                    push_named_symbol(&mut scan.symbols, &tokens, index + 2, "actor enum", 13, &[]);
                } else {
                    push_named_symbol(&mut scan.symbols, &tokens, index + 1, "actor", 7, &[]);
                }
            }
            Some("app") if depth == 0 => {
                push_named_symbol(&mut scan.symbols, &tokens, index + 1, "app", 9, &[]);
            }
            Some("fn") if depth <= 1 => {
                push_callable_symbol(&mut scan.symbols, &tokens, index + 1, "function", 3);
            }
            Some(kind @ ("entry" | "delegate")) if depth == 1 => {
                push_callable_symbol(&mut scan.symbols, &tokens, index + 1, kind, 2);
            }
            Some("const") if depth <= 1 => {
                if let Some(name) = tokens[index + 1..]
                    .iter()
                    .take_while(|token| !token.is_symbol('=') && !token.is_symbol(';'))
                    .filter_map(EditorToken::ident)
                    .last()
                {
                    scan.symbols.push(ScannedSymbol {
                        name: name.to_string(),
                        detail: format!("const {name}"),
                        kind: 21,
                        params: Vec::new(),
                    });
                }
            }
            _ => {}
        }

        if token.is_symbol('{') {
            depth += 1;
        } else if token.is_symbol('}') {
            depth = depth.saturating_sub(1);
        }
        index += 1;
    }

    scan
}

fn push_named_symbol(
    symbols: &mut Vec<ScannedSymbol>,
    tokens: &[EditorToken],
    name_index: usize,
    declaration_kind: &str,
    completion_kind: u8,
    params: &[String],
) {
    if let Some(name) = tokens.get(name_index).and_then(EditorToken::ident) {
        symbols.push(ScannedSymbol {
            name: name.to_string(),
            detail: format!("{declaration_kind} {name}"),
            kind: completion_kind,
            params: params.to_vec(),
        });
    }
}

fn push_callable_symbol(
    symbols: &mut Vec<ScannedSymbol>,
    tokens: &[EditorToken],
    name_index: usize,
    declaration_kind: &str,
    completion_kind: u8,
) {
    let Some(name) = tokens.get(name_index).and_then(EditorToken::ident) else {
        return;
    };
    let params = callable_params(tokens, name_index + 1);
    symbols.push(ScannedSymbol {
        name: name.to_string(),
        detail: format!("{declaration_kind} {name}({})", params.join(", ")),
        kind: completion_kind,
        params,
    });
}

fn callable_params(tokens: &[EditorToken], mut index: usize) -> Vec<String> {
    while index < tokens.len() && !tokens[index].is_symbol('(') {
        if tokens[index].is_symbol('{') || tokens[index].is_symbol(';') {
            return Vec::new();
        }
        index += 1;
    }
    if index == tokens.len() {
        return Vec::new();
    }

    index += 1;
    let mut params = Vec::new();
    let mut segment = Vec::new();
    let mut nested = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.is_symbol('(') || token.is_symbol('[') || token.is_symbol('<') {
            nested += 1;
        } else if token.is_symbol(')') {
            if nested == 0 {
                if let Some(name) = segment.iter().filter_map(EditorToken::ident).next_back() {
                    params.push(name.to_string());
                }
                break;
            }
            nested -= 1;
        } else if token.is_symbol(']') || token.is_symbol('>') {
            nested = nested.saturating_sub(1);
        } else if token.is_symbol(',') && nested == 0 {
            if let Some(name) = segment.iter().filter_map(EditorToken::ident).next_back() {
                params.push(name.to_string());
            }
            segment.clear();
            index += 1;
            continue;
        }
        segment.push(token.clone());
        index += 1;
    }
    params
}

fn completion_items(uri: &str, source: &str, open_documents: &HashMap<String, String>, prefix: &str) -> Vec<Value> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    let mut visited = HashSet::new();
    for (symbol, local) in collect_symbols(uri, source, open_documents, &mut visited) {
        if !seen.insert(symbol.name.clone()) {
            continue;
        }
        let mut item = json!({
            "label": symbol.name,
            "kind": symbol.kind,
            "detail": symbol.detail,
            "sortText": format!("{}-{}", if local { 0 } else { 1 }, symbol.name),
        });
        if !symbol.params.is_empty() || matches!(symbol.kind, 2 | 3) {
            item["insertText"] = Value::String(call_snippet(&symbol.name, &symbol.params));
            item["insertTextFormat"] = json!(2);
        }
        items.push(item);
    }

    for primitive in PRIMITIVE_TYPES {
        if seen.insert((*primitive).to_string()) {
            items.push(json!({
                "label": primitive,
                "kind": 25,
                "detail": "Argent primitive type",
                "sortText": format!("2-{primitive}"),
            }));
        }
    }

    for builtin in BUILTINS {
        if seen.insert(builtin.name.to_string()) {
            let insert_name = qualified_insert_name(builtin.name, prefix);
            items.push(json!({
                "label": builtin.name,
                "kind": 3,
                "detail": builtin.signature,
                "documentation": { "kind": "markdown", "value": builtin.documentation },
                "insertText": call_snippet(insert_name, builtin.params),
                "insertTextFormat": 2,
                "sortText": format!("2-{}", builtin.name),
            }));
        }
    }

    for keyword in KEYWORDS {
        if seen.insert((*keyword).to_string()) || *keyword == "state" {
            items.push(json!({
                "label": keyword,
                "kind": 14,
                "detail": "Argent keyword",
                "sortText": format!("3-{keyword}"),
            }));
        }
    }
    items
}

fn qualified_insert_name<'a>(name: &'a str, prefix: &str) -> &'a str {
    let Some((qualifier, _)) = prefix.rsplit_once('.') else {
        return name;
    };
    name.strip_prefix(qualifier).and_then(|suffix| suffix.strip_prefix('.')).unwrap_or(name)
}

fn completion_prefix(source: &str, position: Option<&Value>) -> String {
    let Some(position) = position else {
        return String::new();
    };
    let Some(line) = position.get("line").and_then(Value::as_u64).and_then(|line| usize::try_from(line).ok()) else {
        return String::new();
    };
    let Some(character) = position.get("character").and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok()) else {
        return String::new();
    };
    let Some(line_text) = source.split('\n').nth(line) else {
        return String::new();
    };

    let mut utf16_units = 0usize;
    let mut byte_offset = line_text.len();
    for (offset, character_value) in line_text.char_indices() {
        if utf16_units >= character {
            byte_offset = offset;
            break;
        }
        let next_units = utf16_units + character_value.len_utf16();
        if next_units > character {
            byte_offset = offset;
            break;
        }
        utf16_units = next_units;
    }

    let prefix_start = line_text[..byte_offset]
        .char_indices()
        .rev()
        .find_map(|(offset, character_value)| {
            (!character_value.is_ascii_alphanumeric() && character_value != '_' && character_value != '.').then_some(offset + 1)
        })
        .unwrap_or(0);
    line_text[prefix_start..byte_offset].to_string()
}

fn collect_symbols(
    uri: &str,
    source: &str,
    open_documents: &HashMap<String, String>,
    visited: &mut HashSet<PathBuf>,
) -> Vec<(ScannedSymbol, bool)> {
    let scan = scan_document(source);
    let mut symbols = scan.symbols.into_iter().map(|symbol| (symbol, true)).collect::<Vec<_>>();
    let Some(path) = file_path_from_uri(uri) else {
        return symbols;
    };
    visited.insert(path.clone());

    for import in scan.imports {
        if import == "std::core" {
            symbols.push((
                ScannedSymbol {
                    name: "invocation_uid".to_string(),
                    detail: "fn invocation_uid(byte[] domain) -> byte[32] — std::core".to_string(),
                    kind: 3,
                    params: vec!["domain".to_string()],
                },
                false,
            ));
            continue;
        }
        if import.starts_with("std::") {
            continue;
        }
        let imported_path = if Path::new(&import).is_absolute() {
            PathBuf::from(import)
        } else {
            path.parent().unwrap_or_else(|| Path::new(".")).join(import)
        };
        let imported_path = imported_path.canonicalize().unwrap_or(imported_path);
        if !visited.insert(imported_path.clone()) {
            continue;
        }
        let imported_uri = file_uri(&imported_path);
        let imported_source = open_documents.get(&imported_uri).cloned().or_else(|| std::fs::read_to_string(&imported_path).ok());
        let Some(imported_source) = imported_source else {
            continue;
        };
        symbols.extend(
            collect_symbols(&imported_uri, &imported_source, open_documents, visited).into_iter().map(|(symbol, _)| (symbol, false)),
        );
    }
    symbols
}

fn call_snippet(name: &str, params: &[impl AsRef<str>]) -> String {
    let arguments = params
        .iter()
        .enumerate()
        .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.as_ref()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({arguments})")
}

fn read_uri(uri: &str) -> Option<String> {
    std::fs::read_to_string(file_path_from_uri(uri)?).ok()
}

fn file_path_from_uri(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
        {
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn scans_incomplete_source_for_declarations_and_parameters() {
        let scan = scan_document(
            r#"
                state WalletState { int balance; }
                actor Wallet owns WalletState {
                    entry send(byte[32] recipient, int amount) emits
            "#,
        );

        assert!(scan.symbols.iter().any(|symbol| symbol.name == "WalletState" && symbol.kind == 22));
        assert!(scan.symbols.iter().any(|symbol| symbol.name == "Wallet" && symbol.kind == 7));
        assert_eq!(
            scan.symbols.iter().find(|symbol| symbol.name == "send").map(|symbol| &symbol.params),
            Some(&vec!["recipient".to_string(), "amount".to_string()]),
        );
    }

    #[test]
    fn completion_contains_argent_words_builtins_and_source_symbols() {
        let items = completion_items("untitled:demo", "state WalletState {}", &HashMap::new(), "");
        let labels = items.iter().filter_map(|item| item.get("label").and_then(Value::as_str)).collect::<HashSet<_>>();

        assert!(labels.contains("WalletState"));
        assert!(labels.contains("actor"));
        assert!(labels.contains("this"));
        assert!(labels.contains("tx"));
        assert!(labels.contains("cov_id"));
        assert!(labels.contains("checkSig"));
        assert!(labels.contains("OpTxLockTime"));
        assert!(labels.contains("r0.g16.verify"));
        assert!(!labels.contains("one"));
        assert!(!labels.contains("leader"));
        assert!(!labels.contains("OpSha256"));
        assert_eq!(
            items.iter().filter(|item| item.get("label").and_then(Value::as_str) == Some("state")).count(),
            2,
            "top-level state declarations and the state(input) builtin need separate completion items"
        );
    }

    #[test]
    fn completion_follows_relative_and_standard_imports() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock after epoch").as_nanos();
        let directory = std::env::temp_dir().join(format!("argent-lsp-import-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create import fixture");
        let root = directory.join("root.ag");
        std::fs::write(directory.join("types.ag"), "state ImportedState {}").expect("write imported source");
        let source = "import \"./types.ag\"; import \"std::core\"; app Root {}";
        std::fs::write(&root, source).expect("write root source");

        let items = completion_items(&file_uri(&root), source, &HashMap::new(), "");
        let labels = items.iter().filter_map(|item| item.get("label").and_then(Value::as_str)).collect::<HashSet<_>>();

        assert!(labels.contains("ImportedState"));
        assert!(labels.contains("invocation_uid"));
        std::fs::remove_dir_all(directory).expect("remove import fixture");
    }

    #[test]
    fn dotted_builtin_completion_inserts_only_the_untyped_suffix() {
        let items = completion_items("untitled:demo", "", &HashMap::new(), "r0.g16.");
        let item =
            items.iter().find(|item| item.get("label").and_then(Value::as_str) == Some("r0.g16.verify")).expect("r0.g16 completion");

        assert_eq!(item.get("insertText").and_then(Value::as_str), Some("verify(${1:journalHash}, ${2:proof}, ${3:imageId})"));
        assert_eq!(completion_prefix("🙂 r0.g16.", Some(&json!({ "line": 0, "character": 10 }))), "r0.g16.");
    }

    #[test]
    fn lsp_round_trip_initializes_and_completes() {
        let input = [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": "file:///tmp/demo.ag", "text": "actor Wallet owns WalletState {" } }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/completion",
                "params": { "textDocument": { "uri": "file:///tmp/demo.ag" }, "position": { "line": 0, "character": 5 } }
            }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ];
        let framed = input.iter().map(frame).collect::<String>();
        let mut reader = BufReader::new(Cursor::new(framed.into_bytes()));
        let mut output = Vec::new();

        serve_io(&mut reader, &mut output).expect("serve messages");

        let mut reader = BufReader::new(Cursor::new(output));
        let initialize = read_message(&mut reader).expect("read initialize response").expect("initialize response");
        let completion = read_message(&mut reader).expect("read completion response").expect("completion response");
        let shutdown = read_message(&mut reader).expect("read shutdown response").expect("shutdown response");
        assert_eq!(initialize["id"], 1);
        assert_eq!(completion["id"], 2);
        assert!(
            completion
                .pointer("/result/items")
                .and_then(Value::as_array)
                .is_some_and(|items| { items.iter().any(|item| item["label"] == "Wallet") })
        );
        assert_eq!(shutdown["id"], 3);
    }

    fn frame(value: &Value) -> String {
        let body = serde_json::to_string(value).expect("serialize request");
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }
}
