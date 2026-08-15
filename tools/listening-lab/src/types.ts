export type AnswerKind = 'choice' | 'scale' | 'text'

export interface AuditionWindow {
  pre: number
  post: number
}

export interface PublicTrack {
  alias: string
  label: string
}

export interface ListeningQuestion {
  id: string
  at_seconds: number
  window: AuditionWindow
  question: string
  detail?: string
  kind: AnswerKind
  options?: string[]
  tracks: string[]
  loop?: boolean
  required?: boolean
}

export interface ListeningProtocol {
  schema: 'musializer.listening-test/v1'
  id: string
  title: string
  instructions?: string
  blind: boolean
  tracks: PublicTrack[]
  questions: ListeningQuestion[]
}

export interface ProtocolSummary {
  id: string
  title: string
  blind: boolean
  track_count: number
  question_count: number
  answered_count: number
}

export interface SavedAnswer {
  schema: 'musializer.listening-answer/v1'
  protocol_id: string
  question_id: string
  revision: number
  answer: string | number | null
  note: string
  playhead_seconds: number
  active_track: string
  audition_counts: Record<string, number>
  saved_at: string
}

export type AnswerDraft = Pick<SavedAnswer, 'answer' | 'note'>
