import { useEffect, useMemo, useRef, useState } from 'react'
import { listProtocols, loadAnswers, loadProtocol, saveAnswer } from './api'
import { formatTime } from './time'
import type {
  AnswerDraft,
  ListeningProtocol,
  ProtocolSummary,
  SavedAnswer,
} from './types'
import { WaveformDeck, type DeckState, type WaveformDeckHandle } from './WaveformDeck'

const emptyDeck: DeckState = { time: 0, duration: 0, playing: false, loading: 0, error: '' }

function App() {
  const [protocols, setProtocols] = useState<ProtocolSummary[]>([])
  const [protocol, setProtocol] = useState<ListeningProtocol | null>(null)
  const [answers, setAnswers] = useState(new Map<string, SavedAnswer>())
  const [questionIndex, setQuestionIndex] = useState(0)
  const [activeTrack, setActiveTrack] = useState('A')
  const [deckState, setDeckState] = useState(emptyDeck)
  const [rate, setRate] = useState(1)
  const [zoom, setZoom] = useState(30)
  const [loop, setLoop] = useState(false)
  const [draft, setDraft] = useState<AnswerDraft>({ answer: null, note: '' })
  const [auditions, setAuditions] = useState<Record<string, number>>({})
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
  const [error, setError] = useState('')
  const [seekText, setSeekText] = useState('00:00.000')
  const deck = useRef<WaveformDeckHandle>(null)

  const question = protocol?.questions[questionIndex]
  const completed = answers.size

  useEffect(() => {
    listProtocols()
      .then((items) => {
        setProtocols(items)
        if (items[0]) return selectProtocol(items[0].id)
      })
      .catch((reason) => setError(String(reason)))
  }, [])

  useEffect(() => {
    setSeekText(formatTime(deckState.time))
  }, [deckState.time])

  useEffect(() => {
    if (!question) return
    const saved = answers.get(question.id)
    setDraft(saved ? { answer: saved.answer, note: saved.note } : { answer: null, note: '' })
    const nextTrack = question.tracks.includes(activeTrack) ? activeTrack : question.tracks[0]
    setActiveTrack(nextTrack)
    setAuditions(Object.fromEntries(question.tracks.map((alias) => [alias, 0])))
    setLoop(Boolean(question.loop))
    setSaveState(saved ? 'saved' : 'idle')
    window.setTimeout(() => deck.current?.seek(Math.max(0, question.at_seconds - question.window.pre)), 0)
  }, [question?.id])

  async function selectProtocol(id: string) {
    try {
      setError('')
      deck.current?.pause()
      const [next, saved] = await Promise.all([loadProtocol(id), loadAnswers(id)])
      setProtocol(next)
      setAnswers(new Map(saved.map((answer) => [answer.question_id, answer])))
      setQuestionIndex(0)
      setActiveTrack(next.questions[0]?.tracks[0] || next.tracks[0].alias)
      setDeckState(emptyDeck)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  function changeQuestion(index: number) {
    if (!protocol) return
    deck.current?.pause()
    setQuestionIndex(Math.min(Math.max(index, 0), protocol.questions.length - 1))
  }

  async function persist(nextDraft: AnswerDraft) {
    if (!protocol || !question) return
    setDraft(nextDraft)
    setSaveState('saving')
    try {
      const saved = await saveAnswer(
        protocol.id,
        question.id,
        nextDraft.answer,
        nextDraft.note,
        deck.current?.getTime() || 0,
        activeTrack,
        auditions,
      )
      setAnswers((current) => new Map(current).set(question.id, saved))
      setSaveState('saved')
    } catch (reason) {
      setSaveState('error')
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  function recordAudition(alias: string) {
    setAuditions((current) => ({ ...current, [alias]: (current[alias] || 0) + 1 }))
  }

  const responseReady = useMemo(() => {
    if (!question) return false
    if (question.kind === 'text') return Boolean(String(draft.answer || '').trim())
    return draft.answer !== null
  }, [question, draft.answer])

  useEffect(() => {
    function keydown(event: KeyboardEvent) {
      const target = event.target as HTMLElement
      if (target.matches('input, textarea, select, button')) return
      if (event.code === 'Space') {
        event.preventDefault()
        void deck.current?.playPause()
      } else if (event.key === ',' || event.key === '.') {
        event.preventDefault()
        const direction = event.key === ',' ? -1 : 1
        deck.current?.seek(deckState.time + direction * (event.shiftKey ? 0.1 : 0.01))
      } else if (event.key === '[' || event.key === ']') {
        if (!question || question.tracks.length < 2) return
        event.preventDefault()
        const current = question.tracks.indexOf(activeTrack)
        const direction = event.key === '[' ? -1 : 1
        const next = question.tracks[(current + direction + question.tracks.length) % question.tracks.length]
        void deck.current?.switchTrack(next)
      } else if (event.key.toLowerCase() === 'r') {
        event.preventDefault()
        void deck.current?.audition(loop)
      } else if (/^[1-7]$/.test(event.key) && question?.kind !== 'text') {
        const option = question?.options?.[Number(event.key) - 1]
        if (option) {
          event.preventDefault()
          void persist({ ...draft, answer: option })
        }
      }
    }
    window.addEventListener('keydown', keydown)
    return () => window.removeEventListener('keydown', keydown)
  }, [activeTrack, deckState.time, draft, loop, question])

  if (!protocol) {
    return (
      <main className="min-h-screen bg-black p-6 font-mono text-[#e5e6e2]">
        <h1 className="text-2xl font-bold">Musializer Listening Lab</h1>
        <p className="mt-4 text-[#a8aaa4]">{error || 'Looking for listening protocols…'}</p>
      </main>
    )
  }

  if (!question) return null

  return (
    <main className="app-shell">
      <header className="masthead">
        <div>
          <p className="product-name">Musializer Listening Lab</p>
          <h1>{protocol.title}</h1>
        </div>
        <div className="session-status">
          <span>{protocol.blind ? 'Blind session' : 'Named tracks'}</span>
          <strong>{completed}/{protocol.questions.length} answered</strong>
        </div>
      </header>

      <div className="workspace">
        <aside className="question-rail">
          <label className="field-label" htmlFor="protocol-select">Session</label>
          <select
            id="protocol-select"
            value={protocol.id}
            onChange={(event) => void selectProtocol(event.target.value)}
          >
            {protocols.map((item) => (
              <option key={item.id} value={item.id}>{item.title}</option>
            ))}
          </select>
          {protocol.instructions && <p className="instructions">{protocol.instructions}</p>}
          <nav aria-label="Questions">
            {protocol.questions.map((item, index) => {
              const saved = answers.has(item.id)
              return (
                <button
                  key={item.id}
                  type="button"
                  className={`question-row ${index === questionIndex ? 'is-current' : ''}`}
                  onClick={() => changeQuestion(index)}
                >
                  <span>{String(index + 1).padStart(2, '0')}</span>
                  <span>{formatTime(item.at_seconds)}</span>
                  <span>{saved ? 'Answered' : item.required === false ? 'Optional' : 'Open'}</span>
                </button>
              )
            })}
          </nav>
        </aside>

        <section className="listening-stage">
          <div className="measurement-bar">
            <div>
              <span className="field-label">Playhead</span>
              <strong data-testid="playhead">{formatTime(deckState.time)}</strong>
            </div>
            <div>
              <span className="field-label">Duration</span>
              <strong>{formatTime(deckState.duration)}</strong>
            </div>
            <div>
              <span className="field-label">Anchor</span>
              <strong>{formatTime(question.at_seconds)}</strong>
            </div>
            <div>
              <span className="field-label">Track</span>
              <strong>{activeTrack}</strong>
            </div>
          </div>

          <div className="wave-panel">
            <WaveformDeck
              ref={deck}
              protocol={protocol}
              question={question}
              activeTrack={activeTrack}
              onActiveTrack={setActiveTrack}
              onState={setDeckState}
              onAudition={recordAudition}
            />
            <div className="track-switcher" role="group" aria-label="Audio tracks">
              {question.tracks.map((alias) => {
                const track = protocol.tracks.find((item) => item.alias === alias)!
                return (
                  <button
                    type="button"
                    key={alias}
                    className={alias === activeTrack ? 'is-active' : ''}
                    aria-pressed={alias === activeTrack}
                    onClick={() => void deck.current?.switchTrack(alias)}
                  >
                    <strong>{alias}</strong>
                    <span>{track.label}</span>
                    <small>{auditions[alias] || 0} auditions</small>
                  </button>
                )
              })}
            </div>
          </div>

          <div className="transport" aria-label="Playback controls">
            <button type="button" className="primary-action" onClick={() => void deck.current?.playPause()}>
              {deckState.playing ? 'Pause' : 'Play'}
            </button>
            <button type="button" onClick={() => void deck.current?.audition(loop)}>Replay window</button>
            <button type="button" onClick={() => deck.current?.seek(deckState.time - 0.1)}>−100 ms</button>
            <button type="button" onClick={() => deck.current?.seek(deckState.time - 0.01)}>−10 ms</button>
            <button type="button" onClick={() => deck.current?.seek(deckState.time + 0.01)}>+10 ms</button>
            <button type="button" onClick={() => deck.current?.seek(deckState.time + 0.1)}>+100 ms</button>
            <label className="time-entry">
              <span className="field-label">Seek</span>
              <input
                aria-label="Seek time in seconds or mm:ss.mmm"
                value={seekText}
                onChange={(event) => setSeekText(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== 'Enter') return
                  const parts = seekText.split(':').map(Number)
                  const seconds = parts.length === 2 ? parts[0] * 60 + parts[1] : parts[0]
                  if (Number.isFinite(seconds)) deck.current?.seek(seconds)
                }}
              />
            </label>
          </div>

          <div className="fine-controls">
            <label>
              <span className="field-label">Speed</span>
              <select value={rate} onChange={(event) => {
                const value = Number(event.target.value)
                setRate(value)
                deck.current?.setRate(value)
              }}>
                <option value={0.5}>0.50×</option>
                <option value={0.75}>0.75×</option>
                <option value={1}>1.00×</option>
                <option value={1.25}>1.25×</option>
              </select>
            </label>
            <label>
              <span className="field-label">Waveform zoom</span>
              <input
                aria-label="Waveform zoom"
                type="range"
                min="10"
                max="400"
                step="10"
                value={zoom}
                onChange={(event) => {
                  const value = Number(event.target.value)
                  setZoom(value)
                  deck.current?.setZoom(value)
                }}
              />
              <output>{zoom} px/s</output>
            </label>
            <label className="check-control">
              <input type="checkbox" checked={loop} onChange={(event) => setLoop(event.target.checked)} />
              Loop audition window
            </label>
            <p className="shortcuts">Space play · R window · ,/. ±10 ms · Shift ,/. ±100 ms · [/] switch track</p>
          </div>

          <article className="feedback-card">
            <header>
              <span>Question {questionIndex + 1} of {protocol.questions.length}</span>
              <span>{formatTime(Math.max(0, question.at_seconds - question.window.pre))}–{formatTime(question.at_seconds + question.window.post)}</span>
            </header>
            <h2>{question.question}</h2>
            {question.detail && <p className="question-detail">{question.detail}</p>}

            {question.kind === 'text' ? (
              <textarea
                aria-label="Answer"
                value={String(draft.answer || '')}
                onChange={(event) => setDraft({ ...draft, answer: event.target.value })}
                placeholder="Write what you heard and include exact times where useful."
              />
            ) : (
              <div className="answer-grid" role="group" aria-label="Answer choices">
                {question.options?.map((option, index) => (
                  <button
                    type="button"
                    key={option}
                    className={draft.answer === option ? 'is-selected' : ''}
                    aria-pressed={draft.answer === option}
                    onClick={() => void persist({ ...draft, answer: option })}
                  >
                    <span>{index + 1}</span>
                    {option}
                  </button>
                ))}
              </div>
            )}

            <label className="note-field">
              <span className="field-label">Notes and timestamps</span>
              <textarea
                value={draft.note}
                onChange={(event) => {
                  setDraft({ ...draft, note: event.target.value })
                  setSaveState('idle')
                }}
                placeholder="Optional context for the agent."
              />
            </label>

            <footer>
              <div className={`save-state is-${saveState}`} role="status">
                {saveState === 'saving' && 'Saving…'}
                {saveState === 'saved' && 'Saved to the append-only answer log'}
                {saveState === 'error' && 'Could not save'}
                {saveState === 'idle' && (answers.has(question.id) ? 'Notes have unsaved changes' : 'Not answered')}
              </div>
              <div className="footer-actions">
                <button type="button" onClick={() => changeQuestion(questionIndex - 1)} disabled={questionIndex === 0}>Previous</button>
                <button type="button" onClick={() => void persist(draft)} disabled={!responseReady}>Save feedback</button>
                <button type="button" className="primary-action" onClick={() => changeQuestion(questionIndex + 1)} disabled={questionIndex === protocol.questions.length - 1}>Next</button>
              </div>
            </footer>
          </article>
          {error && <p className="error-banner">{error}</p>}
        </section>
      </div>
    </main>
  )
}

export default App
