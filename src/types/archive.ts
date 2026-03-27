// Rust の ArchiveRow に対応する型（snake_case で受け渡し）
export interface ArchiveRow {
  id: string;
  name: string;
  repository_id: string;
  repository_name: string;
  path: string;
  branch_name: string;
  archived_at: number;
}

export interface ArchiveListResult {
  items: ArchiveRow[];
  has_more: boolean;
}
