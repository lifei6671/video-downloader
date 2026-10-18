import { invoke, Channel } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type Status = 'parsing' | 'ready' | 'queued' | 'downloading' | 'completed' | 'failed' | 'canceled';
export type Resolver = 'auto' | 'yt-dlp' | 'dtk';
export type ProjectPage = 'source' | 'issues' | 'releases' | 'license' | 'credits' | 'credit-ytdlp' | 'credit-dtk' | 'credit-tauri' | 'credit-react' | 'credit-shadcn' | 'credit-lucide';
export interface UpdateCheck { current_version: string; latest_version: string; release_tag: string; has_update: boolean; release_notes: string; published_at: string | null; installer: { name: string; size: number } | null }
export interface UpdateDownload { id: string; revision: number; release: UpdateCheck; phase: 'preparing' | 'downloading' | 'verifying' | 'canceling' | 'completed' | 'canceled' | 'failed'; downloaded_bytes: number; total_bytes: number | null; filename: string | null; error: string | null }
export interface RuntimeTool { id: 'ffmpeg' | 'javascript'; available: boolean; provider: string | null; version: string | null; path: string | null; source: 'system' | 'managed' | null; can_install: boolean }
export interface RuntimeStatus { platform: string; directory: string; tools: RuntimeTool[] }
export interface RuntimeProgress { tool: RuntimeTool['id']; phase: 'downloading' | 'verifying' | 'installing'; downloaded_bytes: number; total_bytes: number }
export interface ImageMeta { width?: number | null; height?: number | null; quality_source: 'original' | 'high_definition'; preview_filename?: string; filename?: string; bytes?: number; sha256?: string }
export interface VideoFormat { requires_merge?: boolean; format_id: string; container: string; codec?: string | null; width?: number | null; height?: number | null; filesize?: number | null; filesize_approx?: number | null; fps?: number | null; tbr?: number | null }
export interface Video { cookie_warning?: string; formats?: VideoFormat[]; selected_format_id?: string | null; platform: string; video_id: string; canonical_url: string; title?: string; author?: string; cover_url?: string; duration_ms?: number; width?: number | null; height?: number | null; filesize?: number | null; filesize_approx?: number | null; kind?: 'video' | 'image_album'; resolver?: Resolver; images?: ImageMeta[]; music?: { title?: string | null; author?: string | null } }
export interface Task { merging?: boolean; id: string; revision: number; status: Status; video: Video | null; bytes: number; total_bytes: number | null; speed_bps: number | null; error: string | null; path: string | null; created_at: number; completed_at: number | null; canceling: boolean; file_missing: boolean; queue_order: number | null; download_requested: boolean; cover_path: string | null; cover_error: string | null; files: ImageMeta[]; completed_items: number | null; total_items: number | null; cache_hit: boolean; cached_at: number | null; cover_cached: boolean }
export interface Profile { platform: string; user_agent: string; proxy: string | null; cookie_configured: boolean }
export interface Settings { download_directory: string; max_downloads: number; profiles: Profile[] }
export const api = {
  readClipboardText: () => invoke<string>('read_clipboard_text'),
  openProjectPage: (page: ProjectPage, releaseTag?: string) => invoke<void>('open_project_page', { page, releaseTag }),
  checkForUpdates: () => invoke<UpdateCheck>('check_for_updates'),
  updateDownload: () => invoke<UpdateDownload | null>('get_update_download'),
  startUpdateDownload: (releaseTag: string) => invoke<UpdateDownload>('start_update_download', { releaseTag }),
  cancelUpdateDownload: (downloadId: string) => invoke<UpdateDownload>('cancel_update_download', { downloadId }),
  openUpdateInstaller: (downloadId: string) => invoke<void>('open_update_installer', { downloadId }),
  subscribeUpdateDownload: (callback: (status: UpdateDownload) => void) => listen<UpdateDownload>('update_download', event => callback(event.payload)),
  checkRuntimeDependencies: () => invoke<RuntimeStatus>('check_runtime_dependencies'),
  installRuntimeDependency: (tool: RuntimeTool['id'], progress: (value: RuntimeProgress) => void) => {
    const onProgress = new Channel<RuntimeProgress>();
    onProgress.onmessage = progress;
    return invoke<RuntimeStatus>('install_runtime_dependency', { tool, onProgress });
  },
  cancelRuntimeDependency: () => invoke<void>('cancel_runtime_dependency'),
  cookie: (platform: string) => invoke<string>('get_profile_cookie', { platform }),
  image: (taskId: string, index: number) => invoke<number[]>('get_task_image', { taskId, index }),
  cover: (taskId: string) => invoke<number[]>('get_task_cover', { taskId }),
  tasks: () => invoke<Task[]>('list_tasks'),
  settings: () => invoke<Settings>('get_settings'),
  resolve: (input: string, resolver: Resolver = 'auto') => invoke<Task>('resolve_video', { input, resolver }),
  chooseFormat: (taskId: string, formatId: string) => invoke<Task>('choose_video_format', { taskId, formatId }),
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
  DEPENDENCY_BUSY: '依赖检测或下载正在进行，可取消后重新检测',
  DEPENDENCY_UNSUPPORTED_SYSTEM: '当前系统或架构暂不支持自动下载，请自行安装后重新检测',
  DEPENDENCY_NETWORK_ERROR: '依赖下载失败，请检查网络或系统代理后重试',
  DEPENDENCY_TIMEOUT: '依赖下载或验证超时，请稍后重试',
  DEPENDENCY_CHECKSUM_MISMATCH: '下载文件校验不一致，未安装，请重新下载',
  DEPENDENCY_INVALID_ARCHIVE: '下载的压缩包无效，未安装，请重新下载',
  DEPENDENCY_VALIDATION_FAILED: '依赖无法正常执行或版本不符，未确认安装成功',
  CANCELED: '依赖操作已取消',
  OPEN_LINK_FAILED: '无法打开系统浏览器，请稍后重试',
  UPDATE_TIMEOUT: '检查更新超时，请检查网络后重试',
  UPDATE_NETWORK_ERROR: '无法连接 GitHub，请检查网络或系统代理后重试',
  UPDATE_RATE_LIMITED: 'GitHub 访问受限或请求过于频繁，请稍后重试',
  UPDATE_NO_RELEASE: '未找到正式发行版本，请稍后再试',
  UPDATE_SERVICE_ERROR: 'GitHub 更新服务暂时不可用，请稍后重试',
  UPDATE_INVALID_RESPONSE: '更新信息格式异常，请稍后重试或查看发行版本',
  UPDATE_INVALID_VERSION: '无法识别当前应用版本，请查看发行版本',
  UPDATE_CHECK_REQUIRED: '请先检查更新，再下载安装包',
  UPDATE_DOWNLOAD_BUSY: '已有更新正在下载，请查看进度或取消后重试',
  UPDATE_DOWNLOAD_SHUTTING_DOWN: '软件正在退出，请重新启动后再下载更新',
  UPDATE_UNSUPPORTED_SYSTEM: '当前系统或架构暂无自动下载包，请前往发行页面',
  UPDATE_NO_INSTALLER: '此版本没有可验证的适用安装包，请前往发行页面',
  UPDATE_DOWNLOAD_NOT_FOUND: '更新下载已不存在，请重新检查更新',
  UPDATE_DOWNLOAD_FAILED: '安装包下载失败，请检查网络或系统代理后重试',
  UPDATE_DOWNLOAD_TIMEOUT: '安装包下载超时，请稍后重试',
  UPDATE_SIZE_MISMATCH: '安装包大小不完整，未保存，请重新下载',
  UPDATE_CHECKSUM_MISMATCH: '安装包校验失败，请重新下载',
  UPDATE_INSTALLER_NOT_READY: '安装包尚未准备好，请重新下载',
  UPDATE_INSTALLER_MISSING: '安装包已移动或删除，请重新下载',
  UPDATE_INSTALLER_OPEN_FAILED: '无法打开安装包，请稍后重试',
  INVALID_INPUT: '请粘贴抖音、TikTok 或 YouTube 的视频分享链接',
  UNSUPPORTED_PLATFORM: '请粘贴抖音、TikTok 或 YouTube 的视频分享链接',
  FFMPEG_UNAVAILABLE: '此高清规格需要合并音视频，请在系统设置的运行依赖中检测并下载 FFmpeg。',
  JS_RUNTIME_UNAVAILABLE: 'YouTube 解析需要 Node.js 22+ 或 Deno 2.3+，请在系统设置的运行依赖中检测并下载。',
  BROWSER_COOKIES_UNAVAILABLE: '未从系统默认浏览器读取到此平台的可用 Cookie。请确认已登录；若浏览器被占用或加密不支持，可关闭后重试，或在设置中粘贴 Cookie。',
  MEDIA_MERGE_FAILED: 'FFmpeg 合并音视频失败，请检查本机 FFmpeg 和磁盘空间后重试。',
  FORMAT_UNAVAILABLE: '所选视频规格已不可用，请重新解析后选择；未自动降级。',
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
