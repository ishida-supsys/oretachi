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
