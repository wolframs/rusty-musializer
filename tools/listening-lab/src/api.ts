import type { ListeningProtocol, ProtocolSummary, SavedAnswer } from './types'

async function json<T>(input: RequestInfo | URL, init?: RequestInit): Promise<T> {
  const response = await fetch(input, init)
  if (!response.ok) {
    const body = await response.json().catch(() => ({ error: response.statusText }))
    throw new Error(body.error || response.statusText)
  }
  return response.json() as Promise<T>
}

export const listProtocols = () => json<ProtocolSummary[]>('/api/protocols')

export const loadProtocol = (id: string) =>
  json<ListeningProtocol>(`/api/protocol?protocol=${encodeURIComponent(id)}`)

export const loadAnswers = (id: string) =>
  json<SavedAnswer[]>(`/api/answers?protocol=${encodeURIComponent(id)}`)

export const audioUrl = (protocolId: string, alias: string) =>
  `/api/audio?protocol=${encodeURIComponent(protocolId)}&track=${encodeURIComponent(alias)}`

export async function saveAnswer(
  protocolId: string,
  questionId: string,
  answer: string | number | null,
  note: string,
  playheadSeconds: number,
  activeTrack: string,
  auditionCounts: Record<string, number>,
): Promise<SavedAnswer> {
  return json<SavedAnswer>('/api/answers', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      protocol_id: protocolId,
      question_id: questionId,
      answer,
      note,
      playhead_seconds: playheadSeconds,
      active_track: activeTrack,
      audition_counts: auditionCounts,
    }),
  })
}
