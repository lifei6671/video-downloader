import { invoke, Channel, convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type Status = 'parsing' | 'ready' | 'queued' | 'downloading' | 'completed' | 'failed' | 'canceled';
export type Resolver = 'auto' | 'yt-dlp' | 'dtk' | 'grammers';
export type ProjectPage = 'source' | 'issues' | 'releases' | 'license' | 'credits' | 'credit-ytdlp' | 'credit-dtk' | 'credit-tauri' | 'credit-react' | 'credit-shadcn' | 'credit-lucide';
export interface UpdateCheck { current_version: string; latest_version: string; release_tag: string; has_update: boolean; release_notes: string; published_at: string | null; installer: { name: string; size: number } | null }
export interface UpdateDownload { id: string; revision: number; release: UpdateCheck; phase: 'preparing' | 'downloading' | 'verifying' | 'canceling' | 'completed' | 'canceled' | 'failed'; downloaded_bytes: number; total_bytes: number | null; filename: string | null; error: string | null }
export type ModelVariant = 'cpu' | 'cuda';
export interface RuntimeTool { id: 'ffmpeg' | 'javascript' | 'fun-asr-nano'; available: boolean; provider: string | null; version: string | null; path: string | null; source: 'system' | 'managed' | null; can_install: boolean; download_bytes?: number | null; variants?: ModelVariant[]; variant_download_bytes?: Partial<Record<ModelVariant, number>>; variant_model_bytes?: Partial<Record<ModelVariant, number>> }
export interface VideoTranscript { task_id: string; operation_id: string; sequence: number; state: 'running' | 'completed' | 'no_speech' | 'failed' | 'canceled' | 'interrupted'; phase: string | null; backend?: 'cpu' | 'cuda' | null; completed_segments?: number | null; total_segments?: number | null; error_code: string | null; updated_at: number; has_result: boolean; source_changed: boolean; result: {text: string; source: {path: string; size: number; mtime_ns: string}; model_revision: string; model_name?: string | null; pipeline_revision: string; backend?: 'cpu' | 'cuda' | null; completed_at: number} | null }
export interface RuntimeStatus { platform: string; directory: string; tools: RuntimeTool[] }
export interface RuntimeProgress { tool: RuntimeTool['id']; phase: 'downloading' | 'verifying' | 'installing'; downloaded_bytes: number; total_bytes: number }
export interface ImageMeta { width?: number | null; height?: number | null; quality_source: 'original' | 'high_definition'; preview_filename?: string; filename?: string; bytes?: number; sha256?: string }
export interface VideoFormat { requires_merge?: boolean; format_id: string; container: string; codec?: string | null; width?: number | null; height?: number | null; filesize?: number | null; filesize_approx?: number | null; fps?: number | null; tbr?: number | null }
export interface AuthorProfile { id?: string | null; nickname?: string | null; handle?: string | null; kind: 'user' | 'channel'; avatar_cached?: boolean }
export interface Video { author_profile?: AuthorProfile; cookie_warning?: string; formats?: VideoFormat[]; selected_format_id?: string | null; platform: string; video_id: string; canonical_url: string; title?: string; author?: string; cover_url?: string; duration_ms?: number; width?: number | null; height?: number | null; filesize?: number | null; filesize_approx?: number | null; kind?: 'video' | 'image_album'; resolver?: Resolver; images?: ImageMeta[]; music?: { title?: string | null; author?: string | null } }
export interface Task { insertion_id?: number; asset_id?: string | null; total_bytes_estimate?: number | null; telegram_link?: string | null; eta_seconds?: number | null; merging?: boolean; id: string; revision: number; status: Status; video: Video | null; bytes: number; total_bytes: number | null; speed_bps: number | null; error: string | null; path: string | null; created_at: number; download_requested_at?: number | null; completed_at: number | null; canceling: boolean; file_missing: boolean; queue_order: number | null; download_requested: boolean; duplicate_confirmed?: boolean; cover_path: string | null; cover_error: string | null; files: ImageMeta[]; completed_items: number | null; total_items: number | null; cache_hit: boolean; cached_at: number | null; cover_cached: boolean }
export interface Profile { http_chunk_size?: number | null; platform: string; user_agent: string; proxy: string | null; cookie_configured: boolean }
export interface Settings { download_directory: string; filename_template: string; max_downloads: number; profiles: Profile[] }
export interface FilenamePreview { path: string; segments: { text: string; variable: string | null }[] }
export type TelegramPurpose = 'download' | 'backup';
export interface BackupUpdate { telegram?: { account_id: string; channel_id: string } | null; create_library: boolean; cleanup_deleted_records: boolean; deleted_record_retention_days: number; display_name?: string; provider_type: 's3' | 'telegram'; profile: 'r2' | 's3' | 'telegram'; endpoint: string; bucket: string; prefix: string; region: string; path_style: boolean; enabled: boolean; credentials?: { access_key_id: string; secret_access_key: string } }
export interface BackupSettings extends Omit<BackupUpdate, 'credentials' | 'create_library'> { telegram_channel_title?: string | null; can_delete_owned_media?: boolean; space_id: string; library_id: string; credentials_configured: boolean; connection_pending?: boolean; last_error: string | null }
export interface BackupConflict { image: number[] | null; id: string; field: string; value: string; device_id: string; can_adopt: boolean }
export interface BackupLocalData { title: string; transcript: string | null; pending: boolean; remote_deleted: boolean; conflicts: BackupConflict[] }
export interface DeleteActivity { operation_id: string; companion: string | null; step: string }
export interface BackupOperation { operation_id: string; space_id: string; asset_id: string | null; kind: string; state: string; phase: string; payload: { completed_bytes?: number; transfer_started?: boolean; local_cleanup_done?: boolean; archive_next?: string; cleanup_dependency?: string; cleanup_error?: string; cleanup_blocked?: boolean }; attempt: number; last_error: string | null }
export interface BackupLocation { space_id: string; profile: 'r2' | 's3' | 'telegram'; display_name: string; channel_title: string | null }
export interface BackupAsset { in_current_space?: boolean; backup_locations?: BackupLocation[]; asset_id: string; task_id: string | null; metadata: Partial<Video>; original_filename: string; content_sha256: string | null; content_bytes: number; asset_state: 'active' | 'deleted'; path: string | null; local_state: string; space_id: string; remote_state: string; has_cover: boolean; has_transcript: boolean; remote_revision: number; remote_deleted: boolean; has_pending_local_changes: boolean; conflict_count: number }
export interface AssetTranscript { asset_id: string; content_sha256: string | null; text: string; model_revision: string; model_name: string | null; pipeline_revision: string; completed_at: number }
export interface TelegramStorageStatus { configured: boolean; operation_id: string | null; phase: 'disconnected' | 'signed_out' | 'connecting' | 'qr' | 'password' | 'ready'; account: { id: string; name: string } | null; last_error: string | null; qr: boolean[][] | null; qr_expires_at: number | null }
export interface TelegramStorageChannel { id: string; title: string; can_post: boolean; can_delete: boolean }
export const api = {
  telegramStorageStatus: (purpose: TelegramPurpose = 'backup') => invoke<TelegramStorageStatus>('telegram_storage_status', { purpose }),
  telegramStorageResume: (purpose: TelegramPurpose = 'backup') => invoke<TelegramStorageStatus>('telegram_storage_resume', { purpose }),
  telegramStorageLoginQr: (purpose: TelegramPurpose = 'backup', otherAccount = false) => invoke<TelegramStorageStatus>('telegram_storage_login_qr', { purpose, otherAccount }),
  telegramStoragePassword: (operationId: string, password: string, purpose: TelegramPurpose = 'backup') => invoke<TelegramStorageStatus>('telegram_storage_password', { operationId, password, purpose }),
  telegramStorageCancel: (operationId: string, purpose: TelegramPurpose = 'backup') => invoke<TelegramStorageStatus>('telegram_storage_cancel', { operationId, purpose }),
  telegramStorageLogout: (operationId: string, purpose: TelegramPurpose = 'backup') => invoke<TelegramStorageStatus>('telegram_storage_logout', { operationId, purpose }),
  telegramStorageChannels: (operationId: string, purpose: TelegramPurpose = 'backup') => invoke<TelegramStorageChannel[]>('telegram_storage_channels', { operationId, purpose }),
  hideAsset: (assetId: string) => invoke<void>('hide_asset_history', { assetId }),
  releaseAsset: (assetId: string) => invoke<void>('release_local_video', { assetId }),
  deleteAsset: (spaceId: string, assetId: string) => invoke<string>('delete_asset_and_backup', { spaceId, assetId }),
  onBackupAssetDeleted: (callback: (operation: string) => void) =>
    listen<string>('backup_asset_deleted', event => callback(event.payload)),
  ensureAsset: (assetId: string) => invoke<string>('ensure_local_video', { assetId }),
  cancelAssetRestore: (assetId: string) => invoke<void>('cancel_asset_restore', { assetId }),
  assetPlaybackSource: async (assetId: string) => convertFileSrc(await invoke<string>('prepare_asset_playback', { assetId })),
  playAsset: (assetId: string) => invoke<void>('play_asset', { assetId }),
  startAssetTranscription: (assetId: string, force = false) => invoke<VideoTranscript>('start_asset_transcription', { assetId, force }),
  copyAssetTranscript: (assetId: string) => invoke<void>('copy_asset_transcript', { assetId }),
  exportAssetTranscript: (assetId: string) => invoke<string | null>('export_asset_transcript', { assetId }),
  backupAssets: (spaceId: string) => invoke<BackupAsset[]>('list_backup_assets', { spaceId }),
  refreshBackupAssets: (spaceId: string) => invoke<{ imported: number; unchanged: number; rejected: string[] }>('refresh_backup_assets', { spaceId }),
  assetTranscript: (spaceId: string, assetId: string) => invoke<AssetTranscript | null>('get_asset_transcript', { spaceId, assetId }),
  assetCover: (spaceId: string, assetId: string) => invoke<ArrayBuffer>('get_asset_cover', { spaceId, assetId }),
  backupSpaces: () => invoke<BackupSettings[]>('list_backup_spaces'),
  backupSyncEnabled: () => invoke<boolean>('get_backup_sync_enabled'),
  setBackupSyncEnabled: (enabled: boolean) => invoke<boolean>('set_backup_sync_enabled', { enabled }),
  backupSpaceUsage: (spaceId: string) => invoke<{ assets: number; operations: number }>('get_backup_space_usage', { spaceId }),
  deleteBackupSpace: (spaceId: string, confirmed = false) => invoke<void>('delete_backup_space', { spaceId, confirmed }),
  renameBackupSpace: (spaceId: string, displayName: string) => invoke<BackupSettings>('rename_backup_space', { spaceId, displayName }),
  selectBackupSpace: (spaceId: string) => invoke<BackupSettings>('select_backup_space', { spaceId }),
  backupLocalData: (spaceId: string, assetId: string) => invoke<BackupLocalData>('get_backup_local_data', { spaceId, assetId }),
  backupConflict: (spaceId: string, assetId: string, conflictId: string) => invoke<BackupConflict>('get_backup_conflict', { spaceId, assetId, conflictId }),
  resolveBackupConflict: (spaceId: string, assetId: string, conflictId: string, adopt: boolean) => invoke<void>('resolve_backup_conflict', { spaceId, assetId, conflictId, adopt }),
  exportBackupLocalData: (spaceId: string, assetId: string) => invoke<string | null>('export_backup_local_data', { spaceId, assetId }),
  backupSettings: () => invoke<BackupSettings | null>('get_backup_settings'),
  saveBackupSettings: (config: BackupUpdate, activate: boolean) => invoke<BackupSettings>('save_backup_settings', { config: { ...config, activate } }),
  backupCredentials: (spaceId: string) => invoke<{ access_key_id: string; secret_access_key: string }>('get_backup_credentials', { spaceId }),
  testBackupConnection: (config: BackupUpdate) => invoke<void>('test_backup_connection', { config: { ...config, activate: false } }),
  deleteActivity: () => invoke<DeleteActivity | null>('get_backup_delete_activity'),
  onDeleteActivity: (update: (value: DeleteActivity | null) => void) =>
    listen<DeleteActivity | null>('backup_delete_activity', event => update(event.payload)),
  backupOperations: () => invoke<BackupOperation[]>('list_backup_operations'),
  retryBackup: (operationId: string) => invoke<void>('retry_backup_operation', { operationId }),
  startAssetBackup: (spaceId: string, assetId: string) => invoke<void>('start_asset_backup', { spaceId, assetId }),
  cancelBackup: (operationId: string) => invoke<void>('cancel_backup_operation', { operationId }),
  transcripts: () => invoke<VideoTranscript[]>('list_video_transcripts'),
  transcript: (taskId: string) => invoke<VideoTranscript | null>('get_video_transcript', {taskId}),
  startTranscription: (taskId: string, force = false) => invoke<VideoTranscript>('start_video_transcription', {taskId, force}),
  cancelTranscription: (taskId: string, operationId: string) => invoke<void>('cancel_video_transcription', {taskId, operationId}),
  copyTranscript: (taskId: string) => invoke<void>('copy_video_transcript', {taskId}),
  exportTranscript: (taskId: string) => invoke<string | null>('export_video_transcript', {taskId}),
  subscribeTranscripts: (callback: (snapshot: VideoTranscript) => void) => listen<VideoTranscript>('transcription_updated', event => callback(event.payload)),
  proxy: (platform: string) => invoke<string | null>('get_profile_proxy', { platform }),
  readClipboardText: () => invoke<string>('read_clipboard_text'),
  openProjectPage: (page: ProjectPage, releaseTag?: string) => invoke<void>('open_project_page', { page, releaseTag }),
  openReleaseNoteLink: (url: string) => invoke<void>('plugin:opener|open_url', { url }),
  checkForUpdates: () => invoke<UpdateCheck>('check_for_updates'),
  updateDownload: () => invoke<UpdateDownload | null>('get_update_download'),
  startUpdateDownload: (releaseTag: string) => invoke<UpdateDownload>('start_update_download', { releaseTag }),
  cancelUpdateDownload: (downloadId: string) => invoke<UpdateDownload>('cancel_update_download', { downloadId }),
  openUpdateInstaller: (downloadId: string) => invoke<void>('open_update_installer', { downloadId }),
  subscribeUpdateDownload: (callback: (status: UpdateDownload) => void) => listen<UpdateDownload>('update_download', event => callback(event.payload)),
  checkRuntimeDependencies: () => invoke<RuntimeStatus>('check_runtime_dependencies'),
  installRuntimeDependency: (tool: RuntimeTool['id'], progress: (value: RuntimeProgress) => void, variant?: ModelVariant) => {
    const onProgress = new Channel<RuntimeProgress>();
    onProgress.onmessage = progress;
    return invoke<RuntimeStatus>('install_runtime_dependency', { tool, variant, onProgress });
  },
  cancelRuntimeDependency: () => invoke<void>('cancel_runtime_dependency'),
  cookie: (platform: string) => invoke<string>('get_profile_cookie', { platform }),
  image: (taskId: string, index: number) => invoke<number[]>('get_task_image', { taskId, index }),
  authorAvatar: (taskId: string) => invoke<number[]>('get_author_avatar', { taskId }),
  cover: (taskId: string) => invoke<number[]>('get_task_cover', { taskId }),
  saveCover: (taskId: string) => invoke<boolean>('save_task_cover', { taskId }),
  copyCover: (image: ImageData) => invoke<void>('copy_cover_image', { rgba: Array.from(image.data), width: image.width, height: image.height }),
  tasks: () => invoke<Task[]>('list_tasks'),
  settings: () => invoke<Settings>('get_settings'),
  resolve: (input: string, resolver: Resolver = 'auto') => invoke<Task>('resolve_video', { input, resolver }),
  chooseFormat: (taskId: string, formatId: string) => invoke<Task>('choose_video_format', { taskId, formatId }),
  download: (taskId: string, confirmDuplicate = false) => invoke<void>('start_download', { taskId, confirmDuplicate }),
  cancel: (taskId: string) => invoke<void>('cancel_task', { taskId }),
  retry: (taskId: string) => invoke<Task>('retry_task', { taskId }),
  remove: (taskId: string) => invoke<void>('remove_history', { taskId }),
  clearHistory: (platform: string | null = null) => invoke<string[]>('clear_download_history', { platform }),
  deleteFile: (taskId: string) => invoke<void>('delete_local_file', { taskId }),
  concurrency: (value: number) => invoke<Settings>('set_max_downloads', { value }),
  filenameTemplate: (template: string) => invoke<Settings>('set_filename_template', { template }),
  previewFilename: (template: string) => invoke<FilenamePreview>('preview_filename', { template }),
  directory: () => invoke<Settings>('select_download_directory'),
  reveal: (taskId: string) => invoke<void>('reveal_download', { taskId }),
  play: (taskId: string) => invoke<void>('play_download', { taskId }),
  playbackSource: async (taskId: string) => convertFileSrc(await invoke<string>('prepare_video_playback', { taskId })),
  save: (update: { http_chunk_size?: number | null; platform: string; user_agent: string; cookie: string | null; proxy: string | null }) => invoke<Settings>('save_profile', { update }),
  subscribe: async (callback: (task: Task) => void, removed: (id: string, spaceId?: string) => void) => {
    const stopUpdates = await listen<Task>('task_updated', event => callback(event.payload));
    try {
      const stopRemovals = await listen<string | { id: string; space_id: string }>('task_removed', event => {
        const payload = event.payload;
        if (typeof payload === 'string') removed(payload);
        else removed(payload.id, payload.space_id);
      });
      return () => { stopUpdates(); stopRemovals(); };
    } catch (error) { stopUpdates(); throw error; }
  },
};
export function compareTaskInsertion(a: Task, b: Task): number {
  return (b.insertion_id ?? 0) - (a.insertion_id ?? 0) || b.created_at - a.created_at;
}

export function mergeTasks(current: Task[], incoming: Task[], removed: ReadonlySet<string>): Task[] {
  const map = new Map(current.filter(t => !removed.has(t.id)).map(t => [t.id, t]));
  for (const task of incoming) {
    if (removed.has(task.id)) continue;
    if (!map.has(task.id) || map.get(task.id)!.revision <= task.revision) map.set(task.id, task);
  }
  return [...map.values()].sort(compareTaskInsertion);
}
const errors: Record<string, string> = {
  TELEGRAM_APP_NOT_CONFIGURED: '此构建尚未配置 Telegram 应用身份',
  TELEGRAM_RELOGIN_REQUIRED: '登录已失效，请重新扫码',
  TELEGRAM_QR_INVALID: '无法生成登录二维码，请重新扫码',
  TELEGRAM_QR_TIMEOUT: '扫码登录等待超过 5 分钟，连接已停止，请重新扫码',
  TELEGRAM_APP_INVALID: 'Telegram 应用身份无效，请检查构建配置',
  TELEGRAM_SESSION_STORAGE: '无法读写系统凭据中的 Telegram 会话，请检查凭据库权限后重试',
  TELEGRAM_SESSION_INVALID: 'Telegram 会话不完整，请检查系统凭据库',
  TELEGRAM_INVALID_PHONE: '请输入带国家代码的有效手机号',
  TELEGRAM_INVALID_CODE: '验证码无效，请重新输入',
  TELEGRAM_CODE_EXPIRED: '验证码已过期，请取消连接后重新发送',
  TELEGRAM_INVALID_PASSWORD: '两步验证密码错误，请重新输入',
  TELEGRAM_SIGN_UP_REQUIRED: '请先使用 Telegram 官方客户端注册账号',
  TELEGRAM_AUTH_REQUIRED: '请在系统设置 → 平台账号 → Telegram 中恢复登录或重新扫码',
  TELEGRAM_ACCOUNT_MISMATCH: '登录账号与已保存会话不符，请退出后重试',
  TELEGRAM_CHANNEL_FORBIDDEN: '无法访问频道，请检查频道成员与管理权限',
  TELEGRAM_CONNECTION_ERROR: 'Telegram 连接失败，请检查网络后重试',
  TELEGRAM_RPC_ERROR: 'Telegram 拒绝了本次请求，请稍后重试',
  TELEGRAM_TIMEOUT: 'Telegram 请求超时，连接已停止，请重试',
  TELEGRAM_CANCELED: 'Telegram 连接已取消',
  TELEGRAM_STALE_OPERATION: '此登录操作已结束，请刷新账号状态',
  TELEGRAM_INVALID_PHASE: '登录阶段已变化，请重新连接',
  TELEGRAM_NOT_CONNECTED: '请先连接 Telegram 存储账号',
  TELEGRAM_BUSY: 'Telegram 正在处理请求，请稍后重试',
  TELEGRAM_CLOSED: '应用正在退出，Telegram 连接已停止',
  TELEGRAM_INDEX_INVALID: 'Telegram 备份索引损坏，已停止同步',
  TELEGRAM_INDEX_CHANGED: 'Telegram 备份历史被修改或删除，已停止同步',
  TELEGRAM_OPERATION_MISMATCH: 'Telegram 操作记录不一致，已保留本机数据',
  TELEGRAM_SEND_UNCONFIRMED: 'Telegram 发送结果尚未确认，请重试',
  TELEGRAM_ALIAS_IDENTITY_MISMATCH: '链接对应的频道或消息已变化，请重新解析。',
  TELEGRAM_MEDIA_CHANGED: 'Telegram 文件信息已变化，请重新解析或核对备份记录',
  TELEGRAM_MEDIA_MISSING: 'Telegram 备份文件不存在，本机数据已保留',
  TELEGRAM_FILE_REFERENCE_EXPIRED: 'Telegram 文件引用已过期，请重新恢复',
  TELEGRAM_FILE_TOO_LARGE: '文件超过当前 Telegram 账号的上传限制',
  TELEGRAM_UPLOAD_PART_MISSING: 'Telegram 上传文件不完整，请重新上传',
  TELEGRAM_UPLOAD_PART_REJECTED: 'Telegram 未能保存上传数据，请重试',
  TELEGRAM_CDN_UNSUPPORTED: '当前版本暂不支持此 Telegram 文件的下载方式',
  TELEGRAM_LIMIT_UNAVAILABLE: '暂时无法读取 Telegram 上传限制，请重试',
  TELEGRAM_TRANSFER_CANCELED: 'Telegram 传输已取消',
  TELEGRAM_NETWORK_CHANGED: 'Telegram 正在切换网络',
  TELEGRAM_TRANSFER_TIMEOUT: 'Telegram 传输超时，请重试',
  TELEGRAM_RECONNECTING: '正在重连 Telegram，连接恢复后自动继续',
  TELEGRAM_TRANSFER_IO: 'Telegram 文件传输失败，请重试',
  NAMING_INVALID: '请使用相对路径，路径最多 4 段，不含空目录、反斜杠或上级目录',
  TRANSCRIPTION_BUSY: '已有视频正在提取文案，请完成或取消后再开始',
  TRANSCRIPTION_ENGINE_UNAVAILABLE: '语音转录程序不可用，请检查应用安装是否完整',
  TRANSCRIPTION_MODEL_UNAVAILABLE: '语音模型未安装或已损坏，请下载或修复模型',
  TRANSCRIPTION_NO_AUDIO: '视频没有可识别的音轨',
  TRANSCRIPTION_AUDIO_FAILED: '无法提取音频，请检查视频格式、文件完整性和磁盘空间',
  TRANSCRIPTION_FAILED: '语音识别失败，请重新提取；重复失败时可重新下载模型修复',
  TRANSCRIPTION_VAD_FAILED: '语音分段失败，请重试或重新下载模型修复',
  TRANSCRIPTION_CUDA_FAILED: 'CUDA 识别失败，请重试或在运行依赖中改用 CPU 版',
  TRANSCRIPTION_CUDA_UNAVAILABLE: 'CUDA 组件不可用，请在运行依赖中修复或改用 CPU 版',
  CUDA_DRIVER_UNAVAILABLE: '未检测到 NVIDIA CUDA 驱动，请安装驱动或选择 CPU 版',
  CUDA_DRIVER_TOO_OLD: 'NVIDIA 驱动需支持 CUDA 12.4 或更新版本，请更新驱动或选择 CPU 版',
  CUDA_UNSUPPORTED_GPU: '当前显卡不支持此 CUDA 版，请选择 CPU 版',
  CUDA_UNSUPPORTED_SYSTEM: '当前系统或架构不支持 CUDA 版，请选择 CPU 版',
  TRANSCRIPTION_TIMEOUT: '文案提取超时，请重试',
  TRANSCRIPTION_SOURCE_CHANGED: '处理期间视频文件发生变化，本次结果未保存',
  TRANSCRIPTION_OUTPUT_TOO_LARGE: '识别输出超过大小限制，本次结果未保存',
  TRANSCRIPTION_INTERRUPTED: '上次文案提取已中断，可重新提取',
  TRANSCRIPTION_NOT_FOUND: '尚无已保存的文案',
  TRANSCRIPTION_EXPORT_SOURCE: '不能将文案保存到原视频文件，请选择 TXT 文件',
  CLIPBOARD_WRITE_FAILED: '无法写入系统剪贴板，请重试',
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
  BACKUP_PROVIDER_NOT_IMPLEMENTED: '此备份提供商尚未实现',
  BACKUP_PROVIDER_UNAVAILABLE: '此备份提供商尚未实现',
  BACKUP_UNSUPPORTED_PROVIDER: '此备份提供商尚未实现',
  BACKUP_STORAGE_NOT_VALIDATED: '请保存配置以校验备份存储',
  BACKUP_ATOMIC_WRITES_UNSUPPORTED: '此 S3 服务不支持原子条件写入，无法启用多设备同步',
  BACKUP_CONDITIONAL_WRITE_UNSUPPORTED: '此 S3 服务不支持原子条件写入，无法启用多设备同步',
  BACKUP_LOCAL_CHANGES_PENDING: '本机还有未同步变更，请等待确认后再处理冲突',
  BACKUP_CONFLICT_CHANGED: '此冲突已变化，请重新打开版本列表',
  BACKUP_PUBLICATION_UNCONFIRMED: '远端已接收，仍在等待同步确认',
  BACKUP_PROBE_CLEANUP_FAILED: '连接测试的临时对象尚未清理，将在后台重试',
  BACKUP_LOCAL_RELEASED: '本机副本已释放，恢复文件后可重试备份',
  BACKUP_LOCAL_CHANGED: '本机文件已被替换或修改，请检查后重试',
  BACKUP_UNSAFE_FILE: '本机路径不是普通文件，未执行删除',
  BACKUP_SPACE_DISABLED: '备份空间已停用',
  BACKUP_CLEANUP_PENDING: '远端对象尚未清理完成，请重试',
  BACKUP_REUPLOAD_STALE: '删除状态已更新，请重新上传',
  BACKUP_CONDITIONAL_DELETE_UNSUPPORTED: '该存储不支持按版本清理记录，已停止清理',
  BACKUP_INVALID_RETENTION_DAYS: '保留天数须为1到3650之间的整数',
  BACKUP_INVALID_DELETION_INDEX: '删除索引无法校验，清理已停止',
  BACKUP_CLEANUP_UNSUPPORTED: '该提供商暂不支持此删除或清理操作',
  BACKUP_DELETE_UNSUPPORTED: '当前账号没有清理备份文件的权限，请重新连接后重试',
  BACKUP_TOMBSTONE_REQUIRED: '尚未确认远端删除记录，清理已停止',
  BACKUP_MEDIA_IN_USE: '备份文件仍被其他资料引用，清理已停止',
  BACKUP_TIMEOUT: '连接备份存储超时，请检查网络后重试',
  BACKUP_BUSY: '备份存储正在连接，请稍后重试或先停用',
  BACKUP_NETWORK_ERROR: '无法连接远端存储，请检查网络后重试',
  BACKUP_ACCESS_DENIED: '远端存储拒绝访问，请检查凭据和 Bucket 权限',
  BACKUP_INVALID_ACCESS_KEY: 'Access Key ID 无效，请检查备份设置',
  BACKUP_INVALID_SPACE_NAME: '空间名称不能超过 64 个字符或包含控制字符',
  BACKUP_SIGNATURE_MISMATCH: '远端认证失败，请检查 Secret Access Key',
  BACKUP_LIBRARY_NOT_FOUND: '未找到备份库，请检查 Bucket 和前缀',
  BACKUP_LIBRARY_MISMATCH: '备份库身份不一致，请核对存储空间',
  BACKUP_WRITER_CHANGED: '主设备身份已变更，请停止写入并检查备份设置',
  BACKUP_READ_ONLY: '恢复设备仅允许读取远端备份',
  BACKUP_SPACE_INACTIVE: '此备份空间已停用',
  BACKUP_SYNC_DISABLED: '请先启用远端备份',
  BACKUP_SPACE_REFERENCED: '此配置仍有视频或任务引用，请确认后删除',
  BACKUP_CREDENTIALS_MISSING: '请填写备份访问凭据',
  BACKUP_CHECKSUM_MISMATCH: '远端内容校验失败，请重试或检查原备份',
  BACKUP_OBJECT_NOT_FOUND: '远端文件不存在，请刷新备份列表',
  BACKUP_ASSET_DELETED: '视频已从云端删除，无法恢复',
  BACKUP_RATE_LIMITED: '远端请求受限，将稍后重试',
  BACKUP_SERVICE_UNAVAILABLE: '远端服务暂时不可用，请稍后重试',
  BACKUP_INVALID_RECORD: '远端记录格式异常，无法导入',
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
  UPDATE_NEWER_INSTALLER_CACHED: '本地已缓存更高版本安装包，请重新检查更新',
  UPDATE_INSTALLER_MISSING: '安装包已移动或删除，请重新下载',
  UPDATE_INSTALLER_OPEN_FAILED: '无法打开安装包，请稍后重试',
  INVALID_INPUT: '请粘贴抖音、TikTok 或 YouTube 的视频分享链接',
  UNSUPPORTED_PLATFORM: '请粘贴抖音、TikTok 或 YouTube 的视频分享链接',
  FFPROBE_UNAVAILABLE: '读取视频属性需要 FFprobe，请在系统设置的运行依赖中安装 FFmpeg。',
  VIDEO_METADATA_UNAVAILABLE: '无法读取视频时长和尺寸，请检查本地视频是否完整。',
  FFMPEG_UNAVAILABLE: '此高清规格需要合并音视频，请在系统设置的运行依赖中检测并下载 FFmpeg。',
  JS_RUNTIME_UNAVAILABLE: 'YouTube 解析需要 Node.js 22+ 或 Deno 2.3+，请在系统设置的运行依赖中检测并下载。',
  BROWSER_COOKIES_UNAVAILABLE: '未从系统默认浏览器读取到此平台的可用 Cookie。请确认已登录；若浏览器被占用或加密不支持，可关闭后重试，或在设置中粘贴 Cookie。',
  MEDIA_MERGE_FAILED: 'FFmpeg 合并音视频失败，请检查本机 FFmpeg 和磁盘空间后重试。',
  FORMAT_UNAVAILABLE: '所选视频规格已不可用，请重新解析后选择；未自动降级。',
  UNSUPPORTED_CONTENT: '此作品或所选引擎不支持解析，图集请使用自动或 DTK',
  CONFIG_INVALID: '配置格式有误，请检查 Cookie、代理或 User-Agent',
  NETWORK_ERROR: '网络连接失败，请检查网络或代理设置',
  ACCESS_DENIED: '平台拒绝访问，请检查 Cookie 或稍后重试',
  RISK_CONTROL: '平台要求验证或暂时限制当前网络出口；请在浏览器打开链接完成验证，或切换 TUN/代理出口后重试。',
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
    TASK_ACTION_STALE: '任务已更新，请重新操作',
  REPARSE_REQUIRED: '请重新粘贴链接解析',
  INVALID_STATE: '当前任务状态不支持此操作',
  ALREADY_DOWNLOADED: '该规格已有下载任务或成功记录，请确认是否重新下载',
  FILE_MISSING: '文件已移动或删除',
  PLAYBACK_OPEN_FAILED: '无法启动系统默认播放器，请检查视频文件关联或播放器安装',
  PLAYBACK_SOURCE_FAILED: '无法读取本地视频，正在尝试系统播放器',
};
export const errorText = (error: unknown) => {
  const wait = /^TELEGRAM_FLOOD_WAIT:(\d+)$/.exec(String(error));
  return wait ? `Telegram 请求受限，请在 ${wait[1]} 秒后重试` : errors[String(error)] ?? '操作失败，请重试';
};
