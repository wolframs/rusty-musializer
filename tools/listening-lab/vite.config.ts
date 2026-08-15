import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { appendFile, mkdir, readFile, readdir, stat } from 'node:fs/promises'
import { extname, isAbsolute, join, resolve } from 'node:path'
import type { IncomingMessage, ServerResponse } from 'node:http'
import type { Connect, Plugin } from 'vite'
import { defineConfig } from 'vite'
import type {
  CompanionWorkflow,
  FeedbackField,
  FeedbackForm,
  FeedbackValue,
} from './src/types.ts'

const root = new URL('.', import.meta.url).pathname
const protocolsDir = resolve(
  process.env.LISTENING_LAB_PROTOCOLS || join(root, 'protocols'),
)
const answersDir = resolve(
  process.env.LISTENING_LAB_ANSWERS || join(root, '../../build/listening-lab/answers'),
)
const MAX_BODY = 64 * 1024
const audioTypes: Record<string, string> = {
  '.mp3': 'audio/mpeg',
  '.wav': 'audio/wav',
  '.flac': 'audio/flac',
  '.ogg': 'audio/ogg',
  '.m4a': 'audio/mp4',
  '.aac': 'audio/aac',
  '.opus': 'audio/ogg',
}

interface SourceTrack {
  id: string
  label: string
  path: string
}

interface SourceQuestion {
  id: string
  at_seconds: number
  window: { pre: number; post: number }
  question: string
  detail?: string
  kind: 'choice' | 'scale' | 'text'
  options?: string[]
  tracks: string[]
  loop?: boolean
  required?: boolean
  feedback?: string | FeedbackForm
}

interface SourceProtocol {
  schema: string
  id: string
  title: string
  instructions?: string
  blind?: boolean
  playback?: 'browser' | 'external'
  companion?: CompanionWorkflow
  feedback_templates?: Record<string, FeedbackForm>
  tracks: SourceTrack[]
  questions: SourceQuestion[]
}

function fail(response: ServerResponse, status: number, error: string) {
  response.writeHead(status, { 'Content-Type': 'application/json' })
  response.end(JSON.stringify({ error }))
}

function sendJson(response: ServerResponse, value: unknown) {
  response.writeHead(200, {
    'Content-Type': 'application/json',
    'Cache-Control': 'no-store',
  })
  response.end(JSON.stringify(value))
}

function protocolFile(id: string): string {
  if (!/^[A-Za-z0-9._-]{1,100}$/.test(id)) throw new Error('invalid protocol id')
  return join(protocolsDir, `${id}.listen.json`)
}

async function readProtocol(id: string): Promise<SourceProtocol> {
  const parsed = JSON.parse(await readFile(protocolFile(id), 'utf8')) as SourceProtocol
  if (parsed.schema !== 'musializer.listening-test/v1') throw new Error('unsupported protocol schema')
  if (parsed.id !== id) throw new Error('protocol filename and id disagree')
  if (!parsed.title || !Array.isArray(parsed.tracks) || parsed.tracks.length < 1 || parsed.tracks.length > 4) {
    throw new Error('protocol needs a title and 1..4 tracks')
  }
  if (!Array.isArray(parsed.questions) || parsed.questions.length < 1 || parsed.questions.length > 100) {
    throw new Error('protocol needs 1..100 questions')
  }
  if (parsed.playback && !['browser', 'external'].includes(parsed.playback)) {
    throw new Error('playback must be browser or external')
  }
  if (parsed.playback === 'external' && (!parsed.companion?.label || !parsed.companion.command)) {
    throw new Error('external playback needs a companion label and command')
  }
  for (const [name, form] of Object.entries(parsed.feedback_templates || {})) {
    if (!/^[A-Za-z0-9._-]{1,100}$/.test(name)) throw new Error('feedback template id is invalid')
    validateFeedbackForm(form, `feedback template ${name}`)
  }
  const trackIds = new Set(parsed.tracks.map((track) => track.id))
  if (trackIds.size !== parsed.tracks.length) throw new Error('track ids must be unique')
  for (const track of parsed.tracks) {
    if (!/^[A-Za-z0-9._-]{1,100}$/.test(track.id) || !track.label || !track.path) {
      throw new Error('every track needs a safe id, label, and path')
    }
  }
  const questionIds = new Set<string>()
  for (const question of parsed.questions) {
    if (!/^[A-Za-z0-9._-]{1,100}$/.test(question.id) || questionIds.has(question.id)) {
      throw new Error('question ids must be unique and filename-safe')
    }
    questionIds.add(question.id)
    if (!question.question || !['choice', 'scale', 'text'].includes(question.kind)) {
      throw new Error(`question ${question.id} has an invalid prompt or kind`)
    }
    if (!Number.isFinite(question.at_seconds) || question.at_seconds < 0) {
      throw new Error(`question ${question.id} has an invalid time`)
    }
    if (!question.window || question.window.pre < 0 || question.window.post <= 0) {
      throw new Error(`question ${question.id} has an invalid audition window`)
    }
    if (!Array.isArray(question.tracks) || question.tracks.length < 1 || question.tracks.some((id) => !trackIds.has(id))) {
      throw new Error(`question ${question.id} names an unknown track`)
    }
    if (question.kind !== 'text' && (!question.options || question.options.length < 2 || question.options.length > 7)) {
      throw new Error(`question ${question.id} needs 2..7 options`)
    }
    if (typeof question.feedback === 'string') {
      if (!parsed.feedback_templates?.[question.feedback]) {
        throw new Error(`question ${question.id} names an unknown feedback template`)
      }
    } else if (question.feedback) {
      validateFeedbackForm(question.feedback, `question ${question.id} feedback`)
    }
  }
  return parsed
}

function validateFeedbackForm(form: FeedbackForm, context: string) {
  if (!form || !Array.isArray(form.fields) || form.fields.length > 12) {
    throw new Error(`${context} needs 0..12 feedback fields`)
  }
  const ids = new Set<string>()
  for (const field of form.fields) {
    if (!/^[A-Za-z0-9._-]{1,100}$/.test(field.id) || ids.has(field.id)) {
      throw new Error(`${context} field ids must be unique and filename-safe`)
    }
    if (!['single', 'multi', 'scale', 'timestamps'].includes(field.type) || !field.label) {
      throw new Error(`${context} field ${field.id} has an invalid type or label`)
    }
    if (field.type !== 'timestamps') {
      if (!Array.isArray(field.options) || field.options.length < 2 || field.options.length > 10) {
        throw new Error(`${context} field ${field.id} needs 2..10 options`)
      }
      const values = new Set(field.options.map((option) => option.value))
      if (
        values.size !== field.options.length ||
        field.options.some((option) => !option.value || !option.label)
      ) {
        throw new Error(`${context} field ${field.id} options need unique values and labels`)
      }
    }
    if (field.show_when) {
      if (!field.show_when.field || !field.show_when.any_of?.length) {
        throw new Error(`${context} field ${field.id} has an invalid condition`)
      }
      if (field.show_when.field !== 'answer' && !ids.has(field.show_when.field)) {
        throw new Error(`${context} field ${field.id} condition must name an earlier field`)
      }
    }
    if (
      field.max_selections !== undefined &&
      (!Number.isSafeInteger(field.max_selections) || field.max_selections < 1 || field.max_selections > 20)
    ) {
      throw new Error(`${context} field ${field.id} has an invalid selection limit`)
    }
    ids.add(field.id)
  }
}

function feedbackFor(protocol: SourceProtocol, question: SourceQuestion): FeedbackForm | undefined {
  if (typeof question.feedback === 'string') return protocol.feedback_templates?.[question.feedback]
  return question.feedback
}

function fieldVisible(
  field: FeedbackField,
  answer: unknown,
  responses: Record<string, unknown>,
): boolean {
  if (!field.show_when) return true
  const value = field.show_when.field === 'answer' ? answer : responses[field.show_when.field]
  if (Array.isArray(value)) return value.some((item) => field.show_when!.any_of.includes(String(item)))
  return field.show_when.any_of.includes(String(value))
}

function normalizeResponses(
  form: FeedbackForm | undefined,
  raw: unknown,
  answer: unknown,
): { responses: Record<string, FeedbackValue>; complete: boolean } {
  const source = raw && typeof raw === 'object' && !Array.isArray(raw)
    ? raw as Record<string, unknown>
    : {}
  const responses: Record<string, FeedbackValue> = {}
  let complete = true
  for (const field of form?.fields || []) {
    if (!fieldVisible(field, answer, responses)) continue
    const value = source[field.id]
    if (value === undefined) {
      complete &&= !field.required
      continue
    }
    if (field.type === 'timestamps') {
      const maximum = field.max_selections || 4
      if (
        !Array.isArray(value) || value.length > maximum ||
        value.some((item) => !Number.isFinite(item) || Number(item) < 0)
      ) throw new Error(`feedback field ${field.id} has invalid timestamps`)
      const normalized = value.map(Number)
      responses[field.id] = normalized
      complete &&= !field.required || normalized.length > 0
      continue
    }
    const allowed = new Set(field.options!.map((option) => option.value))
    if (field.type === 'multi') {
      const maximum = field.max_selections || field.options!.length
      if (
        !Array.isArray(value) || value.length > maximum ||
        value.some((item) => typeof item !== 'string' || !allowed.has(item)) ||
        new Set(value).size !== value.length
      ) throw new Error(`feedback field ${field.id} has invalid selections`)
      responses[field.id] = value as string[]
      complete &&= !field.required || value.length > 0
    } else {
      if (typeof value !== 'string' || !allowed.has(value)) {
        throw new Error(`feedback field ${field.id} has an invalid selection`)
      }
      responses[field.id] = value
    }
  }
  return { responses, complete }
}

function mappingFor(protocol: SourceProtocol): Array<{ alias: string; track: SourceTrack }> {
  const tracks = [...protocol.tracks]
  if (protocol.blind) {
    const bytes = createHash('sha256')
      .update(`${protocol.id}\0${tracks.map((track) => track.id).join('\0')}`)
      .digest()
    for (let index = tracks.length - 1; index > 0; index -= 1) {
      const swap = bytes[index] % (index + 1)
      ;[tracks[index], tracks[swap]] = [tracks[swap], tracks[index]]
    }
  }
  return tracks.map((track, index) => ({
    alias: String.fromCharCode(65 + index),
    track,
  }))
}

function publicProtocol(protocol: SourceProtocol) {
  const mapping = mappingFor(protocol)
  const aliases = new Map(mapping.map(({ alias, track }) => [track.id, alias]))
  return {
    schema: protocol.schema,
    id: protocol.id,
    title: protocol.title,
    instructions: protocol.instructions,
    blind: Boolean(protocol.blind),
    playback: protocol.playback || 'browser',
    companion: protocol.companion,
    tracks: mapping.map(({ alias, track }) => ({
      alias,
      label: protocol.blind ? `Track ${alias}` : track.label,
    })),
    questions: protocol.questions.map((question) => ({
      ...question,
      tracks: question.tracks.map((id) => aliases.get(id)),
      feedback: feedbackFor(protocol, question),
    })),
  }
}

function answerFile(id: string): string {
  return join(answersDir, `${id}.answers.jsonl`)
}

async function readAnswers(id: string): Promise<Record<string, unknown>[]> {
  try {
    const lines = (await readFile(answerFile(id), 'utf8')).split('\n')
    const records: Record<string, unknown>[] = []
    for (const [index, line] of lines.entries()) {
      if (!line) continue
      try {
        records.push(JSON.parse(line) as Record<string, unknown>)
      } catch (error) {
        const isTornTail = index === lines.length - 1
        if (!isTornTail) throw error
      }
    }
    return records
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw error
  }
}

function latestAnswers(lines: Record<string, unknown>[]) {
  const latest = new Map<string, Record<string, unknown>>()
  for (const line of lines) latest.set(String(line.question_id), line)
  return [...latest.values()].map(({ track_mapping: _mapping, ...answer }) => answer)
}

async function body(request: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = []
  let size = 0
  for await (const chunk of request) {
    const value = Buffer.from(chunk)
    size += value.length
    if (size > MAX_BODY) throw new Error('request body is too large')
    chunks.push(value)
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8')) as Record<string, unknown>
}

async function serveAudio(
  request: IncomingMessage,
  response: ServerResponse,
  protocol: SourceProtocol,
  alias: string,
) {
  const selected = mappingFor(protocol).find((row) => row.alias === alias)
  if (!selected) return fail(response, 404, 'unknown track')
  const rawPath = selected.track.path
  const filePath = isAbsolute(rawPath) ? rawPath : resolve(protocolsDir, rawPath)
  const info = await stat(filePath).catch(() => null)
  if (!info) return fail(response, 404, 'declared audio file is unavailable')
  if (!info.isFile()) return fail(response, 404, 'audio path is not a file')
  const contentType = audioTypes[extname(filePath).toLowerCase()] || 'application/octet-stream'
  const range = request.headers.range
  response.setHeader('Accept-Ranges', 'bytes')
  response.setHeader('Cache-Control', 'private, no-store')
  response.setHeader('Content-Type', contentType)
  if (!range) {
    response.writeHead(200, { 'Content-Length': info.size })
    createReadStream(filePath).pipe(response)
    return
  }
  const match = /^bytes=(\d*)-(\d*)$/.exec(range)
  if (!match) return fail(response, 416, 'invalid byte range')
  const start = match[1] ? Number(match[1]) : 0
  const end = match[2] ? Math.min(Number(match[2]), info.size - 1) : info.size - 1
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start > end || start >= info.size) {
    response.writeHead(416, { 'Content-Range': `bytes */${info.size}` })
    response.end()
    return
  }
  response.writeHead(206, {
    'Content-Length': end - start + 1,
    'Content-Range': `bytes ${start}-${end}/${info.size}`,
  })
  createReadStream(filePath, { start, end }).pipe(response)
}

function listeningApi(): Plugin {
  const middleware: Connect.NextHandleFunction = async (request, response, next) => {
    if (!request.url?.startsWith('/api/')) return next()
    try {
      const url = new URL(request.url, 'http://listening.local')
      if (request.method === 'GET' && url.pathname === '/api/protocols') {
        await mkdir(protocolsDir, { recursive: true })
        const files = (await readdir(protocolsDir)).filter((name) => name.endsWith('.listen.json')).sort()
        const protocols = await Promise.all(files.map((name) => readProtocol(name.slice(0, -'.listen.json'.length))))
        const summaries = await Promise.all(protocols.map(async (protocol) => ({
          id: protocol.id,
          title: protocol.title,
          blind: Boolean(protocol.blind),
          track_count: protocol.tracks.length,
          question_count: protocol.questions.length,
          answered_count: latestAnswers(await readAnswers(protocol.id))
            .filter((answer) => answer.complete !== false).length,
        })))
        return sendJson(response, summaries)
      }
      const id = url.searchParams.get('protocol') || ''
      if (request.method === 'GET' && url.pathname === '/api/protocol') {
        return sendJson(response, publicProtocol(await readProtocol(id)))
      }
      if (request.method === 'GET' && url.pathname === '/api/answers') {
        await readProtocol(id)
        return sendJson(response, latestAnswers(await readAnswers(id)))
      }
      if (request.method === 'GET' && url.pathname === '/api/audio') {
        return serveAudio(request, response, await readProtocol(id), url.searchParams.get('track') || '')
      }
      if (request.method === 'POST' && url.pathname === '/api/answers') {
        const value = await body(request)
        const protocolId = String(value.protocol_id || '')
        const questionId = String(value.question_id || '')
        const protocol = await readProtocol(protocolId)
        const question = protocol.questions.find((item) => item.id === questionId)
        if (!question) return fail(response, 400, 'unknown question')
        const mapping = mappingFor(protocol)
        const allowedAliases = question.tracks.map((trackId) => mapping.find((row) => row.track.id === trackId)!.alias)
        const activeTrack = String(value.active_track || '')
        if (!allowedAliases.includes(activeTrack)) return fail(response, 400, 'active track is not part of this question')
        const previous = (await readAnswers(protocolId)).filter((line) => line.question_id === questionId)
        const answer = value.answer ?? null
        if (question.kind === 'text') {
          if (typeof answer !== 'string' || !answer.trim()) return fail(response, 400, 'text answer is empty')
        } else if (typeof answer !== 'string' || !question.options?.includes(answer)) {
          return fail(response, 400, 'answer is not one of this question\'s options')
        }
        const playhead = Number(value.playhead_seconds)
        if (!Number.isFinite(playhead) || playhead < 0) return fail(response, 400, 'playhead is invalid')
        const rawCounts = value.audition_counts
        if (!rawCounts || typeof rawCounts !== 'object' || Array.isArray(rawCounts)) {
          return fail(response, 400, 'audition counts are invalid')
        }
        const auditionCounts = Object.fromEntries(
          allowedAliases.map((alias) => {
            const count = Number((rawCounts as Record<string, unknown>)[alias] || 0)
            return [alias, Number.isSafeInteger(count) && count >= 0 ? count : 0]
          }),
        )
        const structured = normalizeResponses(feedbackFor(protocol, question), value.responses, answer)
        const record = {
          schema: 'musializer.listening-answer/v1',
          protocol_id: protocolId,
          question_id: questionId,
          revision: previous.length + 1,
          answer,
          note: String(value.note || '').slice(0, 10_000),
          responses: structured.responses,
          complete: structured.complete,
          playhead_seconds: playhead,
          active_track: activeTrack,
          audition_counts: auditionCounts,
          saved_at: new Date().toISOString(),
          track_mapping: Object.fromEntries(mapping.map(({ alias, track }) => [alias, track.id])),
        }
        await mkdir(answersDir, { recursive: true, mode: 0o700 })
        await appendFile(answerFile(protocolId), `${JSON.stringify(record)}\n`, { mode: 0o600 })
        const { track_mapping: _mapping, ...publicRecord } = record
        return sendJson(response, publicRecord)
      }
      return fail(response, 404, 'unknown endpoint')
    } catch (error) {
      return fail(response, 400, error instanceof Error ? error.message : 'request failed')
    }
  }
  return {
    name: 'listening-lab-api',
    configureServer(server) {
      server.middlewares.use(middleware)
    },
    configurePreviewServer(server) {
      server.middlewares.use(middleware)
    },
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss(), listeningApi()],
  server: {
    host: '0.0.0.0',
    port: 4178,
    fs: {
      strict: true,
      allow: [root],
      deny: ['protocols/**', '**/*.listen.json', '**/*.answers.jsonl'],
    },
  },
  preview: { host: '0.0.0.0', port: 4178 },
})
