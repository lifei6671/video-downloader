import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type Status = 'parsing' | 'ready' | 'queued' | 'downloading' | 'completed' | 'failed' | 'canceled';
export type Resolver = 'auto' | 'yt-dlp' | 'dtk';
export interface ImageMeta { width?: number | null; height?: number | null; quality_source: 'original' | 'high_definition'; preview_filename?: string; filename?: string; bytes?: number; sha256?: string }
export interface Video { platform: string; video_id: string; canonical_url: string; title?: string; author?: string; cover_url?: string; duration_ms?: number; width?: number | null; height?: number | null; filesize?: number | null; filesize_approx?: number | null; kind?: 'video' | 'image_album'; resolver?: Resolver; images?: ImageMeta[]; music?: { title?: string | null; author?: string | null } }
export interface Task { id: string; revision: number; status: Status; video: Video | null; bytes: number; total_bytes: number | null; speed_bps: number | null; error: string | null; path: string | null; created_at: number; completed_at: number | null; canceling: boolean; file_missing: boolean; queue_order: number | null; download_requested: boolean; cover_path: string | null; cover_error: string | null; files: ImageMeta[]; completed_items: number | null; total_items: number | null }
export interface Profile { platform: string; user_agent: string; proxy: string | null; cookie_configured: boolean }
export interface Settings { download_directory: string; max_downloads: number; profiles: Profile[] }
export const api = {
  cookie: (platform: string) => invoke<string>('get_profile_cookie', { platform }),
  image: (taskId: string, index: number) => invoke<number[]>('get_task_image', { taskId, index }),
  cover: (taskId: string) => invoke<number[]>('get_task_cover', { taskId }),
  tasks: () => invoke<Task[]>('list_tasks'),
  settings: () => invoke<Settings>('get_settings'),
  resolve: (input: string, resolver: Resolver = 'auto') => invoke<Task>('resolve_video', { input, resolver }),
  download: (taskId: string, confirmDuplicate = false) => invoke<void>('start_download', { taskId, confirmDuplicate }),
  cancel: (taskId: string) => invoke<void>('cancel_task', { taskId }),
  retry: (taskId: string) => invoke<Task>('retry_task', { taskId }),
  remove: (taskId: string) => invoke<void>('remove_history', { taskId }),
  deleteFile: (taskId: string) => invoke<void>('delete_local_file', { taskId }),
  concurrency: (value: number) => invoke<Settings>('set_max_downloads', { value }),
  directory: () => invoke<Settings>('select_download_directory'),
  reveal: (taskId: string) => invoke<void>('reveal_download', { taskId }),
  save: (update: { platform: string; user_agent: string; cookie: string | null; proxy: string | null }) => invoke<Settings>('save_profile', { update }),
  subscribe: async (callback: (task: Task) => void, removed: (id: string) => void) => {
    const stopUpdates = await listen<Task>('task_updated', event => callback(event.payload));
    try {
      const stopRemovals = await listen<string>('task_removed', event => removed(event.payload));
      return () => { stopUpdates(); stopRemovals(); };
    } catch (error) { stopUpdates(); throw error; }
  },
};
export function mergeTasks(current: Task[], incoming: Task[], removed: ReadonlySet<string>): Task[] {
  const map = new Map(current.filter(t => !removed.has(t.id)).map(t => [t.id, t]));
  for (const task of incoming) {
    if (removed.has(task.id)) continue;
    if (!map.has(task.id) || map.get(task.id)!.revision <= task.revision) map.set(task.id, task);
  }
  return [...map.values()].sort((a, b) => b.created_at - a.created_at);
}
const errors: Record<string, string> = {
  INVALID_INPUT: '请粘贴抖音或 TikTok 的视频分享链接',
  UNSUPPORTED_PLATFORM: '请粘贴抖音或 TikTok 的视频分享链接',
  UNSUPPORTED_CONTENT: '此作品或所选引擎不支持解析，图集请使用自动或 DTK',
  CONFIG_INVALID: '配置格式有误，请检查 Cookie、代理或 User-Agent',
  NETWORK_ERROR: '网络连接失败，请检查网络或代理设置',
  ACCESS_DENIED: '平台拒绝访问，请检查 Cookie 或稍后重试',
  RISK_CONTROL: '平台暂时限制访问，请稍后重试',
  NOT_FOUND: '当前视频无法访问',
  TIMEOUT: '请求超时，请检查网络后重新解析',
  ENGINE_UNAVAILABLE: '下载引擎不可用，请检查引擎是否已打包',
  ENGINE_PROTOCOL_ERROR: '下载引擎返回异常，请重新启动应用',
  UPSTREAM_CHANGED: '平台响应已变化，暂时无法解析此作品',
  CREDENTIAL_ERROR: '系统凭据存储不可用，配置未能读取或保存',
  CREDENTIAL_CLEANUP_ERROR: '新配置已保存，但旧凭据清理失败',
  DATABASE_ERROR: '无法保存任务记录，请检查磁盘空间并重启应用',
  INTERRUPTED: '上次任务已中断，请重新解析并从头下载',
  IO_ERROR: '文件操作失败，请检查目录权限、文件占用和磁盘空间',
  INCOMPLETE_MEDIA: '作品文件不完整，请重新解析并下载',
  INVALID_MEDIA_CONTENT: '下载内容不是有效的媒体文件',
  INVALID_ENGINE_RESULT: '下载结果校验失败',
  TASK_BUSY: '任务仍在执行或清理，请稍后再试',
  REPARSE_REQUIRED: '请重新粘贴链接解析',
  INVALID_STATE: '当前任务状态不支持此操作',
  ALREADY_DOWNLOADED: '该作品已有成功下载记录，请确认是否重新下载',
  FILE_MISSING: '文件已移动或删除',
};
export const errorText = (error: unknown) => errors[String(error)] ?? '操作失败，请重试';
