import { invoke } from '@tauri-apps/api/core';

export interface McpConnection {
  write_enabled: boolean;
  status: { enabled: boolean; endpoint: string | null; error: string | null };
}
export const mcpApi = {
  connection: () => invoke<McpConnection>('get_mcp_connection'),
  setEnabled: (enabled: boolean) => invoke<McpConnection>('set_mcp_service_enabled', { enabled }),
  setWriteEnabled: (enabled: boolean) => invoke<McpConnection>('set_mcp_write_enabled', { enabled }),
  copyUrl: () => invoke<void>('copy_mcp_connection_url'),
};
