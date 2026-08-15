export type AnswerKind = 'choice' | 'scale' | 'text'
export type FeedbackFieldKind = 'single' | 'multi' | 'scale' | 'timestamps'
export type FeedbackValue = string | string[] | number[]

export interface FeedbackOption {
  value: string
  label: string
  description?: string
}

export interface FeedbackCondition {
  field: 'answer' | string
  any_of: string[]
}

export interface FeedbackField {
  id: string
  type: FeedbackFieldKind
  label: string
  help?: string
  required?: boolean
  options?: FeedbackOption[]
  max_selections?: number
  show_when?: FeedbackCondition
}

export interface FeedbackForm {
  fields: FeedbackField[]
  note?: {
    label?: string
    placeholder?: string
    collapsed?: boolean
  }
}

export interface CompanionWorkflow {
  label: string
  command: string
  help?: string
}

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
  feedback?: FeedbackForm
}

export interface ListeningProtocol {
  schema: 'musializer.listening-test/v1'
  id: string
  title: string
  instructions?: string
  blind: boolean
  playback?: 'browser' | 'external'
  companion?: CompanionWorkflow
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
  responses: Record<string, FeedbackValue>
  complete: boolean
  playhead_seconds: number
  active_track: string
  audition_counts: Record<string, number>
  saved_at: string
}

export type AnswerDraft = Pick<SavedAnswer, 'answer' | 'note' | 'responses'>
