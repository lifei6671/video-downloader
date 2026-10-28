import { invoke } from '@tauri-apps/api/core';

export type McpConfigFormat = 'codex' | 'json';
export interface McpConnection {
  write_enabled: boolean;
  status: { enabled: boolean; endpoint: string | null; error: string | null };
  adapter_path: string | null;
  adapter_error: string | null;
  configs: Record<McpConfigFormat, string> | null;
}
export const mcpApi = {
  connection: () => invoke<McpConnection>('get_mcp_connection'),
  setEnabled: (enabled: boolean) => invoke<McpConnection>('set_mcp_service_enabled', { enabled }),
  setWriteEnabled: (enabled: boolean) => invoke<McpConnection>('set_mcp_write_enabled', { enabled }),
  copyConfig: (format: McpConfigFormat) => invoke<void>('copy_mcp_client_config', { format }),
};
