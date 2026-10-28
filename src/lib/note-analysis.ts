import type { AnalysisInput, LibraryNoteAnalysis, ObservationInput } from './library-api';

export const copywritingFields = { opening: '开头', viewpoint: '观点', evidence: '论据', turn: '转折', ending: '结尾', takeaways: '可借鉴点' } as const;
export const observationDimensions = { visual: '画面', captions: '字幕', transition: '转场', sound: '声音' } as const;
export const observationEvidence = { video: '观看视频', audio: '聆听声音', keyframe: '仅看关键帧', unverified: '待核验' } as const;
export const blankObservation = (text = ''): ObservationInput => ({ dimension: 'visual', evidence: 'unverified', text, clip_id: '', keyframe_id: null });
export function blankAnalysis(kind: string, text = ''): AnalysisInput | undefined {
  if (kind === 'copywriting_analysis') return { kind: 'copywriting', opening: text, viewpoint: '', evidence: '', turn: '', ending: '', takeaways: '' };
  if (kind === 'technique_analysis') return { kind: 'technique', observations: [blankObservation(text)] };
}
export function analysisInput(value: LibraryNoteAnalysis): AnalysisInput {
  const content = value.content;
  if (content.kind === 'copywriting') return { kind: 'copywriting', opening: content.opening, viewpoint: content.viewpoint, evidence: content.evidence, turn: content.turn, ending: content.ending, takeaways: content.takeaways };
  return { kind: 'technique', observations: content.observations.map(({ dimension, evidence, text, clip_id, keyframe_id }) => ({ dimension, evidence, text, clip_id, keyframe_id })) };
}
export function analysisCanSave(content: AnalysisInput) {
  if (content.kind === 'copywriting') return Object.keys(copywritingFields).some(key => content[key as keyof typeof copywritingFields].trim());
  return content.observations.length > 0 && content.observations.every(row => row.text.trim() && row.clip_id && !(row.evidence === 'keyframe' && (row.dimension === 'sound' || !row.keyframe_id)));
}
