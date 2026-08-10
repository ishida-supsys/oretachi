use crate::settings::AppSettings;

/// ホームで起動した Claude Code セッションに SessionStart フック経由で注入する既定プロンプト。
/// 定型操作の実体は `.claude/skills/` 側にあるので、ここは役割とスキルの在り処、
/// および削除に承認を要求することだけを伝える短いものにしている。
pub const DEFAULT_HOME_AGENT_PROMPT: &str = include_str!("../prompts/home-agent.md");

/// ホームに注入するプロンプトを解決する。設定 `homeAgentPrompt` が空なら既定値を使う。
pub fn resolve_home_agent_prompt(settings: &AppSettings) -> String {
    settings
        .home_agent_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_HOME_AGENT_PROMPT)
        .to_string()
}

/// ホームワークツリー（ワークツリー追加先ディレクトリ）の `.claude/skills/` に
/// シードとして書き出すスキルファイルの埋め込みデータ。
/// 各エントリは (skills/ ディレクトリからの相対パス, ファイル内容) のペア。
///
/// `claude_plugin_skills::SKILL_FILES` とは別枠であることに注意:
///   - あちらは全ワークツリーで有効になるプラグイン同梱スキル（読み取り専用の資産）
///   - こちらはホーム固有で、書き出したあとはユーザーが所有・編集するファイル
///     （`setup_home_claude_dir` は既存ファイルを上書きしない）
pub const HOME_SKILL_FILES: &[(&str, &str)] = &[
    (
        "worktree-cleanup/SKILL.md",
        include_str!("../skills-home/worktree-cleanup/SKILL.md"),
    ),
    (
        "worktree-report/SKILL.md",
        include_str!("../skills-home/worktree-report/SKILL.md"),
    ),
    (
        "worktree-assign/SKILL.md",
        include_str!("../skills-home/worktree-assign/SKILL.md"),
    ),
];

/// ホームの `.claude/skills/` に同梱スキルを書き出す。
/// `overwrite == false` のときは既に存在するファイルをスキップし、ユーザーの編集を保護する。
/// 戻り値は実際に書き出したファイル数。
pub fn write_home_skill_files(home_path: &str, overwrite: bool) -> Result<usize, String> {
    let skills_dir = std::path::Path::new(home_path).join(".claude").join("skills");
    let mut written = 0usize;

    for (rel_path, content) in HOME_SKILL_FILES {
        let dest = skills_dir.join(rel_path);
        if !overwrite && dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("Failed to create home skill dir {}: {}", parent.display(), e)
            })?;
        }
        std::fs::write(&dest, content)
            .map_err(|e| format!("Failed to write home skill file {}: {}", dest.display(), e))?;
        written += 1;
    }

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_home_agent_prompt_falls_back_to_default() {
        let settings = AppSettings::default();
        assert_eq!(resolve_home_agent_prompt(&settings), DEFAULT_HOME_AGENT_PROMPT);
    }

    #[test]
    fn test_resolve_home_agent_prompt_treats_blank_as_unset() {
        let settings = AppSettings { home_agent_prompt: Some("   \n ".to_string()), ..Default::default() };
        assert_eq!(resolve_home_agent_prompt(&settings), DEFAULT_HOME_AGENT_PROMPT);
    }

    #[test]
    fn test_resolve_home_agent_prompt_uses_custom_value() {
        let settings = AppSettings { home_agent_prompt: Some("  カスタム指示  ".to_string()), ..Default::default() };
        assert_eq!(resolve_home_agent_prompt(&settings), "カスタム指示");
    }
}
