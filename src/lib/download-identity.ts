import type { Task, Video } from '@/api';

export function sameDownloadVariant(old: Video, video: Video) {
  return old.platform === video.platform && old.video_id === video.video_id
    && (old.kind ?? 'video') === (video.kind ?? 'video')
    && (video.kind === 'image_album' || video.platform === 'telegram'
      || old.selected_format_id == null || video.selected_format_id == null
      || old.selected_format_id === video.selected_format_id);
}

export function sameReplacementVariant(old: Video, video: Video) {
  return sameDownloadVariant(old, video)
    && (video.kind === 'image_album' || video.platform === 'telegram'
      || old.selected_format_id != null && old.selected_format_id === video.selected_format_id);
}

export function previousDownload(tasks: Task[], task: Task) {
  const video = task.video;
  if (!video) return;
  const matches = (old: Task) => old.id !== task.id && !!old.video && sameDownloadVariant(old.video, video);
  const completed = (old: Task) => matches(old) && old.status === 'completed' && !old.file_missing && !!old.path;
  return tasks.find(old => completed(old) && old.video && sameReplacementVariant(old.video, video))
    ?? tasks.find(completed)
    ?? tasks.find(old => matches(old) && (old.status === 'queued' || old.status === 'downloading'
      || old.status === 'parsing' && old.download_requested));
}

export function duplicatePromptKey(task: Task) {
  return JSON.stringify([task.id, task.video?.kind ?? 'video', task.video?.selected_format_id ?? null]);
}
