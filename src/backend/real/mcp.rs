//! This device's tools (MCP servers), per workspace: what Connect a tool
//! writes and what agents' turns here are offered.
//!
//! * `<data dir>/mcp-servers.json`: the servers, per workspace (owner-only,
//!   written atomically). No token is ever in it (`McpServerSpec` skips
//!   `auth` when serializing).
//! * `<data dir>/mcp-reach.json`: which agents may reach each one
//!   (`ligus_runtime::McpReach`, device-local, only ever narrows).
//! * A URL server's bearer token lives in the secret store (scope `term`)
//!   with the origin it was saved for; it is sent only when the server's
//!   URL still has that origin.
//!
//! Turns get them through the shared engine (`TurnHost::mcp_servers`): the
//! model-API and local-model runtimes that run the tool loop. Fail-closed:
//! an unreadable file means no servers, never every server.

use std::collections::BTreeMap;
use std::path::Path;

use ligus_core::sandbox::EgressPolicy;
use ligus_runtime::mcp::{McpServerSpec, McpTransport};
use ligus_runtime::secret_store::{SecretId, SecretStore};
use ligus_runtime::setup::validate::origin;
use ligus_runtime::{McpReach, setup::McpFact};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::SECRET_SCOPE;

pub const FILE: &str = "mcp-servers.json";

/// The servers per workspace.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Catalog(pub BTreeMap<Uuid, Vec<McpServerSpec>>);

impl Catalog {
    /// A missing file is empty; an unreadable or malformed one is an error.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        match std::fs::read(data_dir.join(FILE)) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{FILE}: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{FILE}: {e}")),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        ligus_runtime::private_file::write_private_atomic(&data_dir.join(FILE), &bytes).map_err(|e| e.to_string())
    }

    pub fn servers(&self, ws: Uuid) -> &[McpServerSpec] {
        self.0.get(&ws).map(Vec::as_slice).unwrap_or(&[])
    }
}

fn token_id(ws: Uuid, server: &str) -> SecretId {
    SecretId::new(SECRET_SCOPE, format!("mcp-{ws}-{server}"))
}

/// Save a URL server's token with the origin it is for (one secret:
/// `origin\ntoken`).
pub fn put_token(store: &dyn SecretStore, ws: Uuid, server: &str, url: &str, token: &Zeroizing<String>) -> Result<(), String> {
    let t = ligus_runtime::mcp::normalize_bearer_token(token)?.ok_or("the token is empty")?;
    let value = Zeroizing::new(format!("{}\n{t}", origin(url)));
    store.put(&token_id(ws, server), value.as_bytes()).map_err(|e| e.to_string())
}

pub fn delete_token(store: &dyn SecretStore, ws: Uuid, server: &str) -> Result<(), String> {
    store.delete(&token_id(ws, server)).map_err(|e| e.to_string())
}

/// The token for `server`, only while `url` has the origin it was saved for.
pub fn token_for(store: &dyn SecretStore, ws: Uuid, server: &str, url: &str) -> Option<Zeroizing<String>> {
    let v = store.get_string(&token_id(ws, server)).ok().flatten()?;
    let (o, t) = v.split_once('\n')?;
    (o == origin(url) && !t.is_empty()).then(|| Zeroizing::new(t.to_string()))
}

/// What Connect a tool lists for `ws`.
pub fn facts(data_dir: &Path, store: &dyn SecretStore, ws: Uuid) -> Vec<McpFact> {
    let (Ok(cat), Ok(reach)) = (Catalog::load(data_dir), McpReach::load(data_dir)) else { return Vec::new() };
    cat.servers(ws)
        .iter()
        .map(|s| {
            let (transport, target, has_secret) = match &s.transport {
                McpTransport::Stdio { command, args } => {
                    ("stdio", std::iter::once(command.clone()).chain(args.clone()).collect::<Vec<_>>().join(" "), false)
                }
                McpTransport::Http { url } => ("http", origin(url), token_for(store, ws, &s.id, url).is_some()),
            };
            McpFact { id: s.id.clone(), transport: transport.into(), target, enabled: s.enabled, has_secret, agents: reach.get(ws, &s.id) }
        })
        .collect()
}

/// Add a server (enabled) with its reach: the reach first, so a server
/// that can't be written leaves only a narrowing for nothing.
pub fn add(data_dir: &Path, ws: Uuid, spec: &McpServerSpec, reach: &[Uuid]) -> Result<(), String> {
    let mut cat = Catalog::load(data_dir)?;
    if cat.servers(ws).iter().any(|s| s.id == spec.id) {
        return Err(format!("'{}' is already one of this device's tools", spec.id));
    }
    let mut r = McpReach::load(data_dir)?;
    r.set(ws, &spec.id, reach);
    r.save(data_dir)?;
    let mut spec = spec.clone();
    spec.enabled = true;
    spec.auth = None;
    spec.agents.clear();
    cat.0.entry(ws).or_default().push(spec);
    cat.save(data_dir)
}

/// Remove a server, its reach and its token.
pub fn remove(data_dir: &Path, store: &dyn SecretStore, ws: Uuid, id: &str) -> Result<(), String> {
    let mut cat = Catalog::load(data_dir)?;
    if let Some(v) = cat.0.get_mut(&ws) {
        v.retain(|s| s.id != id);
    }
    cat.save(data_dir)?;
    let mut r = McpReach::load(data_dir)?;
    r.set(ws, id, &[]);
    r.save(data_dir)?;
    delete_token(store, ws, id)
}

/// The servers a turn in `ws` may be offered under `egress`: enabled,
/// allowed by the chat's envelope, with their reach and (for a URL server)
/// the token saved for its origin. None at all when a file can't be read.
pub fn for_turn(data_dir: &Path, store: &dyn SecretStore, ws: Uuid, egress: &EgressPolicy) -> Vec<McpServerSpec> {
    let (Ok(cat), Ok(reach)) = (Catalog::load(data_dir), McpReach::load(data_dir)) else { return Vec::new() };
    let mut servers: Vec<McpServerSpec> = cat.servers(ws).iter().filter(|s| s.enabled && egress.allows_mcp(&s.id)).cloned().collect();
    reach.apply(ws, &mut servers);
    for s in &mut servers {
        s.auth = match &s.transport {
            McpTransport::Http { url } => token_for(store, ws, &s.id, url).map(|t| t.to_string()),
            McpTransport::Stdio { .. } => None,
        };
    }
    servers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_goes_only_to_its_origin_and_never_to_the_file() {
        let d = tempfile::tempdir().unwrap();
        let store = ligus_runtime::secret_store::File0600::new(d.path());
        let ws = Uuid::new_v4();
        let spec = McpServerSpec::new("issues", McpTransport::Http { url: "https://mcp.example.com/v1".into() });
        put_token(&store, ws, "issues", "https://mcp.example.com/v1", &Zeroizing::new("Bearer tok-canary".into())).unwrap();
        add(d.path(), ws, &spec, &[]).unwrap();
        let file = std::fs::read_to_string(d.path().join(FILE)).unwrap();
        assert!(!file.contains("tok-canary"), "{file}");
        let got = for_turn(d.path(), &store, ws, &EgressPolicy::Open);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].auth.as_deref(), Some("tok-canary"));
        assert!(token_for(&store, ws, "issues", "https://evil.example.net/v1").is_none(), "another origin gets nothing");
        assert!(for_turn(d.path(), &store, ws, &EgressPolicy::Closed).is_empty());
        remove(d.path(), &store, ws, "issues").unwrap();
        assert!(token_for(&store, ws, "issues", "https://mcp.example.com/v1").is_none());
    }

    #[test]
    fn a_broken_file_offers_nothing() {
        let d = tempfile::tempdir().unwrap();
        let store = ligus_runtime::secret_store::File0600::new(d.path());
        let ws = Uuid::new_v4();
        add(d.path(), ws, &McpServerSpec::new("fs", McpTransport::Stdio { command: "npx".into(), args: vec![] }), &[]).unwrap();
        std::fs::write(d.path().join(McpReach::FILE), "{not json").unwrap();
        assert!(for_turn(d.path(), &store, ws, &EgressPolicy::Open).is_empty());
    }
}
