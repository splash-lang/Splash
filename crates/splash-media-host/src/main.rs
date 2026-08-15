#![forbid(unsafe_code)]

//! Example host: an agent-authored Splash workflow orchestrating two
//! policy-bound media capabilities.
//!
//! This is the "production host constructs its own reviewed catalog and policy"
//! pattern the README points to — NOT a demo baked into `splash-cli`. It:
//!
//!   1. registers `image.generate` and `media.transcode` as reviewed JSON
//!      capabilities, each with an executable input/output contract;
//!   2. loads a *data-only* LLM-authored workflow draft and plans it;
//!   3. issues one least-privilege capability lease per step (host policy,
//!      supplied as `step:tool:max` arguments);
//!   4. executes the bounded JSON dataflow and prints the audit trail.
//!
//! The Splash source never touches ffmpeg, curl, the API key, or a filesystem
//! path: it names WHAT (a prompt, a filter) and receives an OPAQUE handle. The
//! adapter chooses where bytes land and resolves handles back to real files —
//! the script cannot address the filesystem. Effects here run as ordinary Mac
//! subprocesses because the point is to prove the capability-orchestration
//! architecture end to end; a mobile host swaps these adapters for the
//! in-process bionic bindings behind the identical contract.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};
use splash_capabilities::{
    http_endpoint_catalog::{HttpEndpointMethod, HttpOrigin, HttpOriginCatalog},
    CapabilityLeaseGrant, JsonToolContract, JsonToolRequest, ToolDescriptor, ToolError,
    ToolMetadata, ToolPolicy,
};
use splash_core::{check_syntax_named, ExecutionLimits};
use splash_workflow::{
    mobile::{MobileWorkflowBuilder, MobileWorkflowRuntime},
    WorkflowData, WorkflowDraft, WorkflowPlan, WorkflowStepCapabilityPolicy,
};

/// Opaque-handle -> host-chosen file path. Shared by both adapters so a
/// generated image can feed a transcode WITHOUT the script ever seeing a path.
type Registry = Arc<Mutex<HashMap<String, PathBuf>>>;
type Seq = Arc<Mutex<u64>>;

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err(
            "usage: splash-media-host <draft.json> <input.json> [step:tool:max ...]".to_owned(),
        );
    }
    let draft_src = std::fs::read_to_string(&args[0]).map_err(|error| error.to_string())?;
    let input_src = std::fs::read_to_string(&args[1]).map_err(|error| error.to_string())?;
    let grants = args[2..]
        .iter()
        .map(|value| parse_grant(value))
        .collect::<Result<Vec<_>, _>>()?;

    let dir = PathBuf::from(
        std::env::var("SPLASH_MEDIA_DIR").unwrap_or_else(|_| "/tmp/splash-media".to_owned()),
    );

    // (2a) The check gate: preflight every generated step through the canonical
    // syntax checker BEFORE anything is planned or run. This is where a model's
    // JS/Python reflexes (ternary, one-line block, `=>`, spread) get caught and
    // turned into line-numbered feedback — the "check" half of generate→check→fix.
    // See docs/authoring-for-llms.md for the rules this enforces.
    if !preflight(&draft_src)? {
        println!(
            "\nGenerated source failed the syntax gate; nothing was planned or executed."
        );
        println!("Feed the diagnostics above back to the model, regenerate, and re-run.");
        return Ok(false);
    }

    // (2b) The untrusted plan is data until the host decides to run it.
    let draft = WorkflowDraft::from_json(&draft_src).map_err(|error| {
        format!("draft rejected as data (never executed): {error}")
    })?;
    let input = WorkflowData::from_input_json(&input_src).map_err(|error| error.to_string())?;

    // (1) The host's own reviewed capability catalog.
    let mut workflow = build_runtime(dir)?;
    let plan = workflow
        .plan_draft(draft)
        .map_err(|error| format!("plan/syntax review failed: {error}"))?;

    // (3) One least-privilege lease per step, from host policy.
    let catalog = workflow.tool_catalog();
    let policies = build_policies(&plan, &catalog, &grants)?;

    println!("catalog: {}", capability_names(&catalog).join(", "));
    println!(
        "plan {} — {} step(s); leases granted by host policy:",
        plan.id(),
        plan.steps().len()
    );
    if grants.is_empty() {
        println!("  (none — every effectful step will be denied)");
    }
    for grant in &grants {
        println!("  {} may call {} up to {}x", grant.0, grant.1, grant.2);
    }

    // (4) Execute the bounded JSON dataflow under those leases.
    let approval = workflow
        .approve_dataflow_with_step_capability_policies(&plan, input, policies)
        .map_err(|error| error.to_string())?;
    let outcome = workflow.execute_dataflow(&plan, approval);

    println!("--- audit (every capability call the runtime authorized) ---");
    for event in workflow.audit().iter() {
        println!(
            "  #{} {} -> {:?}  in={}B out={}B",
            event.event_sequence, event.tool, event.outcome, event.input_bytes, event.output_bytes
        );
    }

    match outcome {
        Ok(data) => {
            println!("--- step outputs (opaque handles the host can resolve) ---");
            println!(
                "{}",
                serde_json::to_string_pretty(&data.outputs()).unwrap_or_default()
            );
            Ok(true)
        }
        Err(error) => {
            println!("--- execution stopped: {error} ---");
            println!("--- debug: {error:?} ---");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                println!("--- caused by: {cause} ---");
                source = cause.source();
            }
            println!("(a step called a capability it held no lease for, or an adapter failed)");
            Ok(false)
        }
    }
}

/// The check gate: run each generated step's source through the canonical
/// syntax checker and print line-numbered diagnostics. Returns true iff every
/// step is valid canonical Splash. Effect-free — no capability host, no eval.
fn preflight(draft_json: &str) -> Result<bool, String> {
    let parsed: Value = serde_json::from_str(draft_json).map_err(|error| error.to_string())?;
    let steps = parsed["steps"]
        .as_array()
        .ok_or_else(|| "draft has no `steps` array".to_owned())?;
    println!("--- preflight: canonical syntax check on each generated step ---");
    let mut all_valid = true;
    for step in steps {
        let id = step["id"].as_str().unwrap_or("?");
        let source = step["source"].as_str().unwrap_or("");
        let report = check_syntax_named(id, source, ExecutionLimits::default())
            .map_err(|error| error.to_string())?;
        if report.valid {
            println!("  step '{id}': OK");
        } else {
            all_valid = false;
            println!("  step '{id}': REJECTED");
            for diagnostic in &report.diagnostics {
                println!(
                    "      line {} col {}: {}",
                    diagnostic.line, diagnostic.column, diagnostic.message
                );
            }
        }
    }
    Ok(all_valid)
}

/// Build the host runtime with the two reviewed media capabilities.
fn build_runtime(dir: PathBuf) -> Result<MobileWorkflowRuntime, String> {
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    let seq: Seq = Arc::new(Mutex::new(0));
    // A data-script that enriches a whole chart parses one JSON payload per
    // row; the default 200k instruction budget is spent after ~6 rows, so raise
    // it for chart-sized loops.
    let mut limits = ExecutionLimits::default();
    limits.instruction_limit = 8_000_000;
    // The default script deadline assumes no I/O; a chart of sequential network
    // calls needs a wall-clock budget that covers real request latency.
    limits.soft_timeout = std::time::Duration::from_secs(90);
    limits.hard_timeout = std::time::Duration::from_secs(120);
    let mut builder =
        MobileWorkflowBuilder::with_limits(limits, splash_capabilities::DEFAULT_MAX_PENDING_TOOLS)
            .map_err(|error| error.to_string())?;

    {
        let registry = registry.clone();
        let seq = seq.clone();
        let dir = dir.clone();
        builder
            .register_json_tool(
                ToolPolicy::json("image.generate"),
                ToolMetadata::new(
                    "Generate an image from a text prompt (OpenAI images API); returns an opaque handle.",
                ),
                image_contract()?,
                move |request: &JsonToolRequest| image_generate(request, &registry, &seq, &dir),
            )
            .map_err(|error| error.to_string())?;
    }
    {
        let registry = registry.clone();
        let seq = seq.clone();
        builder
            .register_json_tool(
                ToolPolicy::json("media.transcode"),
                ToolMetadata::new(
                    "Apply a bounded ffmpeg -vf filter to a prior image handle; returns a new handle.",
                ),
                media_contract()?,
                move |request: &JsonToolRequest| media_transcode(request, &registry, &seq, &dir),
            )
            .map_err(|error| error.to_string())?;
    }
    {
        // (1c) A reviewed HTTP *origin* — not a bespoke `sys.movies` fetcher.
        // The host fixes ONE origin (IMDb's keyless suggestion API, HTTPS,
        // GET-only, byte-bounded); the Splash step decides which title to query
        // by building a bounded URL under that origin. This is the whole point:
        // the movie logic lives in the card's data-script, not in Rust.
        let mut imdb = HttpOriginCatalog::default();
        imdb.insert(
            HttpOrigin::https("imdb", HttpEndpointMethod::Get, "https://v3.sg.media-imdb.com")
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let mut imdb_policy = ToolPolicy::json("net.imdb");
        // One data-script enriches a whole chart, so the lease needs a call
        // budget above the default 1; each suggestion payload is a few KB.
        imdb_policy.max_calls = 16;
        imdb_policy.max_output_bytes = 256 * 1024;
        builder
            .register_http_origin_catalog_tool(
                imdb_policy,
                ToolMetadata::new(
                    "IMDb suggestion API — one reviewed HTTPS origin, keyless GET. The script supplies a bounded URL.",
                ),
                imdb,
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(builder.build())
}

fn image_contract() -> Result<JsonToolContract, String> {
    JsonToolContract::new(
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "size": {"type": "string"},
                "quality": {"type": "string"}
            },
            "required": ["prompt"],
            "additionalProperties": false
        }),
        json!({
            "type": "object",
            "properties": {
                "handle": {"type": "string"},
                "bytes": {"type": "integer"},
                "seconds": {"type": "integer"}
            },
            "required": ["handle", "bytes", "seconds"],
            "additionalProperties": false
        }),
    )
    .map_err(|error| error.to_string())
}

fn media_contract() -> Result<JsonToolContract, String> {
    JsonToolContract::new(
        json!({
            "type": "object",
            "properties": {
                "src": {"type": "string"},
                "vf": {"type": "string"},
                "crf": {"type": "integer"}
            },
            "required": ["src", "vf"],
            "additionalProperties": false
        }),
        json!({
            "type": "object",
            "properties": {
                "handle": {"type": "string"},
                "bytes": {"type": "integer"},
                "seconds": {"type": "integer"}
            },
            "required": ["handle", "bytes", "seconds"],
            "additionalProperties": false
        }),
    )
    .map_err(|error| error.to_string())
}

fn image_generate(
    request: &JsonToolRequest,
    registry: &Registry,
    seq: &Seq,
    dir: &Path,
) -> Result<Value, ToolError> {
    let prompt = request.input["prompt"]
        .as_str()
        .ok_or_else(|| ToolError::Denied("image.generate requires a string prompt".to_owned()))?;
    let size = request.input["size"].as_str().unwrap_or("1024x1024");
    let quality = request.input["quality"].as_str().unwrap_or("low");
    let key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| ToolError::Denied("OPENAI_API_KEY not set for image.generate".to_owned()))?;

    std::fs::create_dir_all(dir).map_err(|error| ToolError::Failed(format!("mkdir: {error}")))?;
    let body = json!({"model": "gpt-image-2", "prompt": prompt, "size": size, "quality": quality, "n": 1});
    let req_path = dir.join("request.json");
    let resp_path = dir.join("response.json");
    std::fs::write(&req_path, body.to_string())
        .map_err(|error| ToolError::Failed(format!("write request: {error}")))?;

    let started = Instant::now();
    let status = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            "https://api.openai.com/v1/images/generations",
            "-H",
            "Content-Type: application/json",
            "-H",
            &format!("Authorization: Bearer {key}"),
            "--data-binary",
            &format!("@{}", req_path.display()),
            "-o",
        ])
        .arg(&resp_path)
        .status()
        .map_err(|error| ToolError::Failed(format!("curl spawn: {error}")))?;
    if !status.success() {
        return Err(ToolError::Failed("curl exited non-zero".to_owned()));
    }

    let text = std::fs::read_to_string(&resp_path)
        .map_err(|error| ToolError::Failed(format!("read response: {error}")))?;
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|error| ToolError::Failed(format!("parse response: {error}")))?;
    let b64 = parsed["data"][0]["b64_json"].as_str().ok_or_else(|| {
        let api = parsed["error"]["message"].as_str().unwrap_or("no b64_json in response");
        ToolError::Failed(format!("image API: {api}"))
    })?;
    let png = b64_decode(b64).ok_or_else(|| ToolError::Failed("bad base64".to_owned()))?;

    let n = next_seq(seq);
    let out = dir.join(format!("gen_{n}.png"));
    std::fs::write(&out, &png).map_err(|error| ToolError::Failed(format!("write png: {error}")))?;
    let handle = format!("img_{n}");
    registry
        .lock()
        .map_err(|_| ToolError::Failed("registry poisoned".to_owned()))?
        .insert(handle.clone(), out);
    Ok(json!({"handle": handle, "bytes": png.len(), "seconds": started.elapsed().as_secs()}))
}

fn media_transcode(
    request: &JsonToolRequest,
    registry: &Registry,
    seq: &Seq,
    dir: &Path,
) -> Result<Value, ToolError> {
    let src = request.input["src"]
        .as_str()
        .ok_or_else(|| ToolError::Denied("media.transcode requires a string src handle".to_owned()))?;
    let vf = request.input["vf"]
        .as_str()
        .ok_or_else(|| ToolError::Denied("media.transcode requires a string vf".to_owned()))?;
    // The vf string is untrusted script input: hold it to a bounded filtergraph
    // charset so a card can never smuggle shell/filter escapes past the adapter.
    if vf.is_empty()
        || !vf
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "=:,.-_ ".contains(c))
    {
        return Err(ToolError::Denied(
            "vf has characters outside the allowed filtergraph set".to_owned(),
        ));
    }

    let input_path = registry
        .lock()
        .map_err(|_| ToolError::Failed("registry poisoned".to_owned()))?
        .get(src)
        .cloned()
        .ok_or_else(|| ToolError::Denied(format!("unknown src handle: {src}")))?;

    let n = next_seq(seq);
    let out = dir.join(format!("edit_{n}.png"));
    let started = Instant::now();
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
        .arg(&input_path)
        .args(["-vf", vf])
        .arg(&out)
        .status()
        .map_err(|error| ToolError::Failed(format!("ffmpeg spawn: {error}")))?;
    if !status.success() {
        return Err(ToolError::Failed("ffmpeg exited non-zero".to_owned()));
    }

    let bytes = std::fs::metadata(&out).map(|meta| meta.len()).unwrap_or(0);
    let handle = format!("img_{n}");
    registry
        .lock()
        .map_err(|_| ToolError::Failed("registry poisoned".to_owned()))?
        .insert(handle.clone(), out);
    Ok(json!({"handle": handle, "bytes": bytes, "seconds": started.elapsed().as_secs()}))
}

fn next_seq(seq: &Seq) -> u64 {
    let mut guard = seq.lock().expect("seq mutex");
    *guard += 1;
    *guard
}

fn build_policies(
    plan: &WorkflowPlan,
    catalog: &[ToolDescriptor],
    grants: &[(String, String, usize)],
) -> Result<Vec<WorkflowStepCapabilityPolicy>, String> {
    for (step, tool, _) in grants {
        if !plan.steps().iter().any(|candidate| &candidate.id == step) {
            return Err(format!("grant references a step absent from the draft: {step}"));
        }
        if !catalog.iter().any(|descriptor| &descriptor.name == tool) {
            return Err(format!("grant names a capability absent from the catalog: {tool}"));
        }
    }
    Ok(plan
        .steps()
        .iter()
        .map(|step| {
            WorkflowStepCapabilityPolicy::new(
                step.id.clone(),
                grants
                    .iter()
                    .filter(|(candidate, _, _)| candidate == &step.id)
                    .map(|(_, tool, max)| CapabilityLeaseGrant::new(tool.clone(), *max)),
            )
        })
        .collect())
}

fn capability_names(catalog: &[ToolDescriptor]) -> Vec<String> {
    catalog.iter().map(|descriptor| descriptor.name.clone()).collect()
}

fn parse_grant(value: &str) -> Result<(String, String, usize), String> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 3 {
        return Err(format!("bad grant '{value}', want step:tool:max"));
    }
    let max = parts[2]
        .parse::<usize>()
        .map_err(|_| format!("bad max-calls in grant '{value}'"))?;
    Ok((parts[0].to_owned(), parts[1].to_owned(), max))
}

/// Standard-alphabet base64 decode (image API returns `b64_json`).
fn b64_decode(source: &str) -> Option<Vec<u8>> {
    fn val(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a' + 26)),
            b'0'..=b'9' => Some(u32::from(byte - b'0' + 52)),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = source
        .bytes()
        .filter(|byte| !b" \n\r\t".contains(byte))
        .collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut chunk = [0u32; 4];
    let mut filled = 0;
    for &byte in &bytes {
        if byte == b'=' {
            break;
        }
        chunk[filled] = val(byte)?;
        filled += 1;
        if filled == 4 {
            let word = (chunk[0] << 18) | (chunk[1] << 12) | (chunk[2] << 6) | chunk[3];
            out.extend_from_slice(&[(word >> 16) as u8, (word >> 8) as u8, word as u8]);
            filled = 0;
        }
    }
    match filled {
        0 => {}
        2 => {
            let word = (chunk[0] << 18) | (chunk[1] << 12);
            out.push((word >> 16) as u8);
        }
        3 => {
            let word = (chunk[0] << 18) | (chunk[1] << 12) | (chunk[2] << 6);
            out.extend_from_slice(&[(word >> 16) as u8, (word >> 8) as u8]);
        }
        _ => return None,
    }
    Some(out)
}
