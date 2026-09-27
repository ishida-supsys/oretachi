import type { ArtifactData, ArtifactMeta } from "../types/artifact";

/**
 * ディスク上のアーティファクト JSON は `type` フィールドで content type を持つ
 * （Rust 側は `serde(rename = "type")`）。フロントの型はいずれも `content_type` を
 * 使うため、Tauri / Web どちらのデータアクセス層でもここでマッピングする。
 */
export function mapArtifactMeta(raw: any): ArtifactMeta {
  return { ...raw, content_type: raw.type ?? raw.content_type };
}

export function mapArtifactData(raw: any): ArtifactData {
  return { ...raw, content_type: raw.type ?? raw.content_type };
}
