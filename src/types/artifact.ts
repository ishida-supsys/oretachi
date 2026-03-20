export interface ArtifactMeta {
  id: string;
  title: string;
  content_type: string;
  language?: string;
  created_at: number;
  updated_at: number;
}

export interface ArtifactData extends ArtifactMeta {
  content: string;
}

export interface ArtifactChangedEvent {
  worktreeId: string;
  artifactId: string;
  command: string;
}
