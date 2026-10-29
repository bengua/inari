//! Git forges the local engine's threads publish to: GitHub, Forgejo (or Gitea), as many as the
//! person sets in the interface, none written in the code. A forge is its address, its kind,
//! the account that publishes and, kept apart (`forge_keys.json`, mode 600) and never shown,
//! that account's token. Pushing and opening a pull request are the person's actions in the
//! interface: the model's commands never get the token (the agent produces, the person
//! publishes).

use super::LocalEngine;
use crate::capsule::io;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Forge {
    pub id: String,
    pub name: String,
    /// github or forgejo (Gitea speaks the same).
    pub kind: String,
    /// Its web address: https://github.com, http://atelier:3000.
    pub url: String,
    /// Its API, when not the usual one (GitHub's api.github.com, Forgejo's <url>/api/v1).
    #[serde(default)]
    pub api: String,
    /// The account that publishes (the token's).
    #[serde(default)]
    pub user: String,
    /// Git over SSH instead of HTTP(S), as a prefix: ssh://git@atelier:2222 (the core's keys).
    #[serde(default)]
    pub ssh: String,
}

fn id_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 64
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

/// A token as forges make them: nothing that could end a header or a line of curl's settings.
fn token_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 512
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

/// owner/name, as both forges write a repository.
pub fn repo_like(x: &str) -> bool {
    let mut parts = x.split('/');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [a, b].iter().all(|p| id_like(p) && !p.starts_with('.'))
}

/// A branch name git takes, and nothing a shell or an option could read otherwise.
pub fn branch_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 200
        && !x.starts_with(['-', '/', '.'])
        && !x.contains("..")
        && !x.ends_with(['/', '.'])
        && !x.ends_with(".lock")
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./".contains(c))
}

impl Forge {
    pub fn problem(&self) -> Option<String> {
        if !id_like(&self.id) {
            return Some("a forge's id is letters, digits, - _ and . only".into());
        }
        if !["github", "forgejo"].contains(&self.kind.as_str()) {
            return Some("a forge is GitHub or Forgejo (Gitea speaks as Forgejo)".into());
        }
        for (what, url) in [("address", &self.url), ("API", &self.api)] {
            if (what == "address" || !url.is_empty())
                && !(url.starts_with("https://") || url.starts_with("http://"))
            {
                return Some(format!(
                    "the forge's {what} starts with https:// or http://"
                ));
            }
            if url.contains(char::is_whitespace) {
                return Some(format!("the forge's {what} has a space"));
            }
        }
        if !self.ssh.is_empty() && !self.ssh.starts_with("ssh://") {
            return Some("Git over SSH is written ssh://git@host:port".into());
        }
        if !self.user.is_empty() && !id_like(&self.user) {
            return Some("an account is letters, digits, - _ and . only".into());
        }
        None
    }

    pub fn api_base(&self) -> String {
        if !self.api.is_empty() {
            return self.api.trim_end_matches('/').to_string();
        }
        match self.kind.as_str() {
            "github" if self.url.trim_end_matches('/') == "https://github.com" => {
                "https://api.github.com".into()
            }
            "github" => format!("{}/api/v3", self.url.trim_end_matches('/')),
            _ => format!("{}/api/v1", self.url.trim_end_matches('/')),
        }
    }

    /// Where git reaches a repository of it.
    pub fn clone_url(&self, repo: &str) -> String {
        if self.ssh.is_empty() {
            format!("{}/{repo}.git", self.url.trim_end_matches('/'))
        } else {
            format!("{}/{repo}.git", self.ssh.trim_end_matches('/'))
        }
    }

    /// The environment that gives git the token for this forge's address only (not on a
    /// command line: other users of the machine do not see it).
    fn git_env(&self, token: &str) -> Vec<(String, String)> {
        if token.is_empty() || !self.ssh.is_empty() {
            return Vec::new();
        }
        let user = if self.user.is_empty() {
            "x-access-token"
        } else {
            &self.user
        };
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{token}"));
        vec![
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            (
                "GIT_CONFIG_KEY_0".into(),
                format!("http.{}/.extraHeader", self.url.trim_end_matches('/')),
            ),
            (
                "GIT_CONFIG_VALUE_0".into(),
                format!("Authorization: Basic {basic}"),
            ),
        ]
    }
}

/// git in a folder, with no prompt; its output, or why it failed.
pub(crate) async fn git(
    dir: &Path,
    args: &[&str],
    env: &[(String, String)],
) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(dir)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| format!("git {} took more than 5 min", args.first().unwrap_or(&"")))?
        .map_err(|e| format!("git cannot start ({e}): is it installed on this core?"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(text)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { text } else { err })
    }
}

/// A call to a forge's API through curl (it speaks TLS; the token goes on its standard input).
async fn api(
    forge: &Forge,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let url = format!("{}{path}", forge.api_base());
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "-m",
        "30",
        "-X",
        method,
        "--config",
        "-",
        "-w",
        "\n%{http_code}",
    ]);
    if let Some(b) = body {
        cmd.args(["--data-binary", &b.to_string()]);
    }
    cmd.arg(&url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("curl cannot start ({e}): is it installed on this core?"))?;
    let mut config = String::from(
        "header = \"Accept: application/json\"\nheader = \"User-Agent: inari\"\nheader = \"Content-Type: application/json\"\n",
    );
    if !token.is_empty() {
        config.push_str(&format!("header = \"Authorization: token {token}\"\n"));
    }
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes()).await;
    }
    let out = tokio::time::timeout(Duration::from_secs(40), child.wait_with_output())
        .await
        .map_err(|_| format!("{url} did not answer"))?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "{url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    let code: u16 = code.trim().parse().unwrap_or(0);
    let v: Value = serde_json::from_str(body)
        .unwrap_or(json!({"raw": body.chars().take(300).collect::<String>()}));
    if (200..300).contains(&code) {
        Ok(v)
    } else {
        let said = v
            .get("message")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| v.to_string());
        Err(format!("the forge says HTTP {code}: {said}"))
    }
}

impl LocalEngine {
    pub fn forges(&self) -> Vec<Forge> {
        std::fs::read(self.dir.join("forges.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn forge_keys(&self) -> BTreeMap<String, String> {
        std::fs::read(self.dir.join("forge_keys.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub(crate) fn forge(&self, id: &str) -> Result<(Forge, String), String> {
        let f = self
            .forges()
            .into_iter()
            .find(|f| f.id == id)
            .ok_or_else(|| format!("no forge {id:?}: Settings → the local engine → Git forges"))?;
        let token = self.forge_keys().get(id).cloned().unwrap_or_default();
        Ok((f, token))
    }

    /// The forges as the sheet shows them: whether each has a token, never the token.
    pub fn forges_view(&self) -> Vec<Value> {
        let keys = self.forge_keys();
        self.forges()
            .into_iter()
            .map(|f| {
                let mut v = serde_json::to_value(&f).expect("serializes");
                v["token"] = json!(keys.contains_key(&f.id));
                v
            })
            .collect()
    }

    /// Add or change a forge (an empty token keeps the one kept); the detail of a test.
    pub async fn forge_action(&self, action: &str, p: &Value) -> Result<String, String> {
        let text = |k: &str| {
            p.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let form = || Forge {
            id: text("id"),
            name: Some(text("name"))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| text("id")),
            kind: text("kind"),
            url: text("url").trim_end_matches('/').to_string(),
            api: text("api").trim_end_matches('/').to_string(),
            user: text("user"),
            ssh: text("ssh").trim_end_matches('/').to_string(),
        };
        let mut keys = self.forge_keys();
        match action {
            "forge_add" => {
                let f = form();
                if let Some(why) = f.problem() {
                    return Err(why);
                }
                let mut all = self.forges();
                all.retain(|x| x.id != f.id);
                all.push(f.clone());
                std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
                io::atomic_write(
                    &self.dir.join("forges.json"),
                    &serde_json::to_vec_pretty(&all).expect("serializes"),
                )
                .map_err(|e| e.to_string())?;
                let token = text("token");
                if !token.is_empty() {
                    if !token_like(&token) {
                        return Err("a token is letters, digits and - _ . only".into());
                    }
                    keys.insert(f.id.clone(), token);
                }
                self.save_forge_keys(&keys)?;
                Ok(String::new())
            }
            "forge_remove" => {
                let id = text("id");
                let mut all = self.forges();
                all.retain(|x| x.id != id);
                io::atomic_write(
                    &self.dir.join("forges.json"),
                    &serde_json::to_vec_pretty(&all).expect("serializes"),
                )
                .map_err(|e| e.to_string())?;
                keys.remove(&id);
                self.save_forge_keys(&keys)?;
                Ok(String::new())
            }
            "forge_test" => {
                let f = form();
                if let Some(why) = f.problem() {
                    return Err(why);
                }
                let token = Some(text("token"))
                    .filter(|t| !t.is_empty() && token_like(t))
                    .or_else(|| keys.get(&f.id).cloned())
                    .unwrap_or_default();
                if token.is_empty() {
                    let v = api(
                        &f,
                        "",
                        "GET",
                        if f.kind == "github" {
                            "/meta"
                        } else {
                            "/version"
                        },
                        None,
                    )
                    .await?;
                    return Ok(format!(
                        "The forge answers{}: give it a token to publish",
                        v.get("version")
                            .and_then(Value::as_str)
                            .map(|x| format!(" ({x})"))
                            .unwrap_or_default()
                    ));
                }
                let v = api(&f, &token, "GET", "/user", None).await?;
                Ok(format!(
                    "Signed in as {}",
                    v.get("login").and_then(Value::as_str).unwrap_or("?")
                ))
            }
            other => Err(format!("unknown forge action {other:?}")),
        }
    }

    fn save_forge_keys(&self, keys: &BTreeMap<String, String>) -> Result<(), String> {
        let path = self.dir.join("forge_keys.json");
        io::atomic_write(&path, &serde_json::to_vec(keys).expect("serializes"))
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Bring a forge's repository into a thread's folder (the files already there stay).
    pub async fn clone_into(
        &self,
        workdir: &Path,
        forge: &str,
        repo: &str,
        branch: &str,
    ) -> Result<String, String> {
        if !repo_like(repo) {
            return Err("a repository is written owner/name".into());
        }
        if !branch.is_empty() && !branch_like(branch) {
            return Err(format!("not a branch name: {branch:?}"));
        }
        if workdir.join(".git").exists() {
            return Err("this folder is already a Git repository".into());
        }
        let (f, token) = self.forge(forge)?;
        let env = f.git_env(&token);
        let tmp = workdir.join(".inari-clone");
        let _ = std::fs::remove_dir_all(&tmp);
        let mut args = vec!["clone", "--no-checkout", "--origin", "origin"];
        if !branch.is_empty() {
            args.extend(["--branch", branch]);
        }
        let url = f.clone_url(repo);
        args.extend([url.as_str(), ".inari-clone"]);
        let got = git(workdir, &args, &env).await;
        if let Err(e) = got {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
        std::fs::rename(tmp.join(".git"), workdir.join(".git")).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&tmp);
        git(workdir, &["checkout", "-f", "HEAD", "--", "."], &[]).await?;
        git(workdir, &["reset", "-q"], &[]).await?;
        Ok(format!("{repo} is in the thread's folder"))
    }

    /// Push the folder's branch to a forge and open a pull request; its address.
    pub async fn publish(
        &self,
        workdir: &Path,
        forge: &str,
        repo: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<String, String> {
        if !repo_like(repo) {
            return Err("a repository is written owner/name".into());
        }
        if !branch_like(base) {
            return Err(format!("not a branch name: {base:?}"));
        }
        if title.trim().is_empty() || title.len() > 250 {
            return Err("a pull request has a title (250 characters at most)".into());
        }
        if !workdir.join(".git").exists() {
            return Err("this folder is not a Git repository".into());
        }
        let branch = git(workdir, &["rev-parse", "--abbrev-ref", "HEAD"], &[]).await?;
        if branch == base || branch == "HEAD" {
            return Err(format!(
                "make a branch first: the work is on {branch}, the pull request goes into {base}"
            ));
        }
        let (f, token) = self.forge(forge)?;
        if token.is_empty() {
            return Err(format!("{} has no token: add it in its settings", f.name));
        }
        let url = f.clone_url(repo);
        let refspec = format!("HEAD:refs/heads/{branch}");
        git(
            workdir,
            &["push", url.as_str(), refspec.as_str()],
            &f.git_env(&token),
        )
        .await?;
        let head = branch.clone();
        let v = api(
            &f,
            &token,
            "POST",
            &format!("/repos/{repo}/pulls"),
            Some(&json!({"title": title.trim(), "head": head, "base": base, "body": body})),
        )
        .await?;
        Ok(v.get("html_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forge_is_checked_and_nothing_is_assumed() {
        let gh = Forge {
            id: "github".into(),
            name: "GitHub".into(),
            kind: "github".into(),
            url: "https://github.com".into(),
            ..Default::default()
        };
        assert_eq!(gh.problem(), None);
        assert_eq!(gh.api_base(), "https://api.github.com");
        assert_eq!(
            gh.clone_url("bengua/inari"),
            "https://github.com/bengua/inari.git"
        );
        let home = Forge {
            id: "atelier".into(),
            name: "Forgejo".into(),
            kind: "forgejo".into(),
            url: "http://atelier:3000".into(),
            ssh: "ssh://git@atelier:2222".into(),
            ..Default::default()
        };
        assert_eq!(home.api_base(), "http://atelier:3000/api/v1");
        assert_eq!(
            home.clone_url("ben/site"),
            "ssh://git@atelier:2222/ben/site.git"
        );
        assert!(home.git_env("t").is_empty(), "over SSH, the core's keys");
        let env = gh.git_env("t0k");
        assert_eq!(env[1].1, "http.https://github.com/.extraHeader");
        assert!(!env[2].1.contains("t0k"), "encoded, not written as is");
        assert!(Forge {
            kind: "svn".into(),
            ..gh.clone()
        }
        .problem()
        .is_some());
        assert!(Forge {
            url: "github.com".into(),
            ..gh
        }
        .problem()
        .is_some());
        assert!(token_like("ghp_abc-1.x") && !token_like("a\"\nheader = x") && !token_like("a b"));
        assert!(repo_like("bengua/inari") && !repo_like("../x") && !repo_like("a/b/c"));
        assert!(
            branch_like("feat/x-1")
                && !branch_like("-f")
                && !branch_like("a..b")
                && !branch_like("x y")
        );
    }
}
