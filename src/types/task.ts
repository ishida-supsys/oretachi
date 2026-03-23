export type AddWorktreeTaskCode = {
  type: "add_worktree";
  repository: string;
  branch: string;
  source_branch?: string;
};

export type AgentWorktreeTaskCode = {
  type: "agent_worktree";
  repository: string;
  branch: string;
  prompt: string;
  remoteExec?: boolean;
};

export type TaskCode = AddWorktreeTaskCode | AgentWorktreeTaskCode;
export type TaskProcessCode = { code: TaskCode[] };

export type TaskStepStatus = "pending" | "running" | "done" | "error";

export interface TaskStep {
  code: TaskCode;
  status: TaskStepStatus;
  error?: string;
}

export type TaskStatus = "generating" | "queued" | "executing" | "completed" | "error";

export interface TaskItem {
  id: string;
  prompt: string;
  createdAt: number;
  status: TaskStatus;
  steps: TaskStep[];
  error?: string;
}
