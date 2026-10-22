import type { Task, Video } from '@/api';

export function sameDownloadVariant(old: Video, video: Video) {
  return old.platform === video.platform && old.video_id === video.video_id
    && (old.kind ?? 'video') === (video.kind ?? 'video')
    && (video.kind === 'image_album' || video.platform === 'telegram'
      || old.selected_format_id == null || video.selected_format_id == null
      || old.selected_format_id === video.selected_format_id);
}

export function previousDownload(tasks: Task[], task: Task) {
  return task.video && tasks.find(old => old.id !== task.id && old.status === 'completed'
    && !old.file_missing && !!old.path && old.video && sameDownloadVariant(old.video, task.video!));
}

export function duplicatePromptKey(task: Task) {
  return JSON.stringify([task.id, task.video?.kind ?? 'video', task.video?.selected_format_id ?? null]);
}
