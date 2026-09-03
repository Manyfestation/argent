use zed_extension_api::{self as zed, settings::LspSettings};

struct ArgentExtension;

impl zed::Extension for ArgentExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        // `lsp.argent-language-server.binary` in Zed settings lets a checkout run a
        // locally built compiler without installing it on PATH.
        let binary = LspSettings::for_worktree(language_server_id.as_ref(), worktree).ok().and_then(|settings| settings.binary);
        let command =
            binary.as_ref().and_then(|binary| binary.path.clone()).or_else(|| worktree.which("argentc")).ok_or_else(|| {
                "`argentc` was not found on PATH. Install it with `cargo install --path /path/to/argent --locked` \
                 or set `lsp.argent-language-server.binary.path` in Zed settings."
                    .to_string()
            })?;
        let args = binary.as_ref().and_then(|binary| binary.arguments.clone()).unwrap_or_else(|| vec!["lsp".to_string()]);
        let env = binary.and_then(|binary| binary.env).map(|env| env.into_iter().collect()).unwrap_or_default();
        Ok(zed::Command { command, args, env })
    }
}

zed::register_extension!(ArgentExtension);
