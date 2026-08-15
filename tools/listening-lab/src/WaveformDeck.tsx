import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from 'react'
import WaveSurfer from 'wavesurfer.js'
import Hover from 'wavesurfer.js/dist/plugins/hover.esm.js'
import Regions from 'wavesurfer.js/dist/plugins/regions.esm.js'
import Timeline from 'wavesurfer.js/dist/plugins/timeline.esm.js'
import { audioUrl } from './api'
import { formatTime } from './time'
import type { ListeningProtocol, ListeningQuestion } from './types'

export interface DeckState {
  time: number
  duration: number
  playing: boolean
  loading: number
  error: string
}

export interface WaveformDeckHandle {
  playPause(): Promise<void>
  pause(): void
  seek(time: number): void
  switchTrack(alias: string): Promise<void>
  audition(loop: boolean): Promise<void>
  setRate(rate: number): void
  setZoom(pixelsPerSecond: number): void
  getTime(): number
}

interface Props {
  protocol: ListeningProtocol
  question: ListeningQuestion
  activeTrack: string
  onActiveTrack: (alias: string) => void
  onState: (state: DeckState) => void
  onAudition: (alias: string) => void
}

const initialState: DeckState = {
  time: 0,
  duration: 0,
  playing: false,
  loading: 0,
  error: '',
}

export const WaveformDeck = forwardRef<WaveformDeckHandle, Props>(function WaveformDeck(
  { protocol, question, activeTrack, onActiveTrack, onState, onAudition },
  ref,
) {
  const containers = useRef(new Map<string, HTMLDivElement>())
  const waves = useRef(new Map<string, WaveSurfer>())
  const regions = useRef(new Map<string, Regions>())
  const activeRef = useRef(activeTrack)
  const questionRef = useRef(question)
  const loopRef = useRef(false)
  const auditionEnd = useRef<number | null>(null)
  const [state, setState] = useState(initialState)
  const stateRef = useRef(initialState)

  const publish = (patch: Partial<DeckState>) => {
    const next = { ...stateRef.current, ...patch }
    stateRef.current = next
    setState(next)
    onState(next)
  }

  useEffect(() => {
    activeRef.current = activeTrack
  }, [activeTrack])

  useEffect(() => {
    questionRef.current = question
    loopRef.current = Boolean(question.loop)
    auditionEnd.current = null
    for (const [alias, plugin] of regions.current) {
      plugin.clearRegions()
      const wave = waves.current.get(alias)
      if (!wave || !wave.getDuration()) continue
      for (const [index, item] of protocol.questions.entries()) {
        if (!item.tracks.includes(alias)) continue
        plugin.addRegion({
          id: `marker-${item.id}`,
          start: item.at_seconds,
          content: String(index + 1),
          color: item.id === question.id ? 'rgba(255,184,0,0.30)' : 'rgba(255,184,0,0.12)',
          drag: false,
          resize: false,
        })
      }
      if (question.tracks.includes(alias)) {
        plugin.addRegion({
          id: `window-${question.id}`,
          start: Math.max(0, question.at_seconds - question.window.pre),
          end: Math.min(wave.getDuration(), question.at_seconds + question.window.post),
          color: 'rgba(255,184,0,0.10)',
          drag: false,
          resize: false,
        })
      }
    }
  }, [protocol.questions, question])

  useEffect(() => {
    const created: WaveSurfer[] = []
    const subscriptions: Array<() => void> = []
    for (const track of protocol.tracks) {
      const container = containers.current.get(track.alias)
      if (!container) continue
      const regionPlugin = Regions.create()
      const wave = WaveSurfer.create({
        container,
        url: audioUrl(protocol.id, track.alias),
        height: 176,
        waveColor: '#565951',
        progressColor: '#ffb800',
        cursorColor: '#fff4cc',
        cursorWidth: 2,
        normalize: false,
        minPxPerSec: 30,
        dragToSeek: true,
        autoScroll: true,
        autoCenter: true,
        backend: 'MediaElement',
        plugins: [
          regionPlugin,
          Hover.create({
            lineColor: '#fff4cc',
            labelColor: '#000000',
            labelBackground: '#ffb800',
            labelSize: 12,
            formatTimeCallback: (time) => formatTime(time),
          }),
          Timeline.create({
            height: 28,
            style: {
              color: '#a8aaa4',
              fontFamily: 'IBM Plex Mono, JetBrains Mono, monospace',
              fontSize: '10px',
            },
            formatTimeCallback: (time) => formatTime(time, time < 10 ? 1 : 0),
          }),
        ],
      })
      wave.setVolume(1)
      waves.current.set(track.alias, wave)
      regions.current.set(track.alias, regionPlugin)
      created.push(wave)
      subscriptions.push(
        wave.on('loading', (loading) => {
          if (activeRef.current === track.alias) publish({ loading })
        }),
        wave.on('ready', (duration) => {
          if (activeRef.current === track.alias) {
            const item = questionRef.current
            wave.setTime(Math.max(0, item.at_seconds - item.window.pre))
            publish({
              time: wave.getCurrentTime(),
              duration,
              loading: 100,
              error: '',
            })
          }
          const plugin = regions.current.get(track.alias)
          if (!plugin) return
          for (const [index, item] of protocol.questions.entries()) {
            if (!item.tracks.includes(track.alias)) continue
            plugin.addRegion({
              id: `marker-${item.id}`,
              start: item.at_seconds,
              content: String(index + 1),
              color: item.id === questionRef.current.id ? 'rgba(255,184,0,0.30)' : 'rgba(255,184,0,0.12)',
              drag: false,
              resize: false,
            })
          }
          const current = questionRef.current
          if (current.tracks.includes(track.alias)) {
            plugin.addRegion({
              id: `window-${current.id}`,
              start: Math.max(0, current.at_seconds - current.window.pre),
              end: Math.min(duration, current.at_seconds + current.window.post),
              color: 'rgba(255,184,0,0.10)',
              drag: false,
              resize: false,
            })
          }
        }),
        wave.on('timeupdate', (time) => {
          if (activeRef.current !== track.alias) return
          const end = auditionEnd.current
          if (end !== null && time >= end) {
            if (loopRef.current) {
              const item = questionRef.current
              wave.setTime(Math.max(0, item.at_seconds - item.window.pre))
            } else {
              wave.pause()
              auditionEnd.current = null
            }
          }
          publish({ time })
        }),
        wave.on('play', () => {
          if (activeRef.current === track.alias) publish({ playing: true })
        }),
        wave.on('pause', () => {
          if (activeRef.current === track.alias) publish({ playing: false })
        }),
        wave.on('error', (error) => {
          if (activeRef.current === track.alias) publish({ error: error.message, loading: 0 })
        }),
      )
    }
    return () => {
      subscriptions.forEach((unsubscribe) => unsubscribe())
      created.forEach((wave) => wave.destroy())
      waves.current.clear()
      regions.current.clear()
    }
    // The deck is recreated only for a different protocol.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [protocol.id])

  useImperativeHandle(ref, () => ({
    async playPause() {
      const wave = waves.current.get(activeRef.current)
      if (wave) await wave.playPause()
    },
    pause() {
      waves.current.get(activeRef.current)?.pause()
    },
    seek(time) {
      const wave = waves.current.get(activeRef.current)
      if (!wave) return
      wave.setTime(Math.min(Math.max(0, time), wave.getDuration()))
      publish({ time: wave.getCurrentTime() })
    },
    async switchTrack(alias) {
      if (alias === activeRef.current) return
      const current = waves.current.get(activeRef.current)
      const target = waves.current.get(alias)
      if (!target) return
      const time = current?.getCurrentTime() || 0
      const wasPlaying = current?.isPlaying() || false
      current?.pause()
      target.setTime(Math.min(time, target.getDuration()))
      activeRef.current = alias
      onActiveTrack(alias)
      onAudition(alias)
      publish({
        time: target.getCurrentTime(),
        duration: target.getDuration(),
        playing: false,
        loading: target.getDuration() ? 100 : 0,
        error: '',
      })
      if (wasPlaying) await target.play()
    },
    async audition(loop) {
      const wave = waves.current.get(activeRef.current)
      if (!wave) return
      const item = questionRef.current
      const start = Math.max(0, item.at_seconds - item.window.pre)
      auditionEnd.current = Math.min(wave.getDuration(), item.at_seconds + item.window.post)
      loopRef.current = loop
      wave.setTime(start)
      onAudition(activeRef.current)
      await wave.play()
    },
    setRate(rate) {
      for (const wave of waves.current.values()) wave.setPlaybackRate(rate, true)
    },
    setZoom(pixelsPerSecond) {
      for (const wave of waves.current.values()) wave.zoom(pixelsPerSecond)
    },
    getTime() {
      return waves.current.get(activeRef.current)?.getCurrentTime() || 0
    },
  }))

  return (
    <div className="wave-deck" aria-label="Audio waveforms">
      {protocol.tracks.map((track) => (
        <div
          key={track.alias}
          className={`wave-layer ${track.alias === activeTrack ? 'is-active' : ''}`}
          data-testid={`waveform-${track.alias}`}
          ref={(element) => {
            if (element) containers.current.set(track.alias, element)
            else containers.current.delete(track.alias)
          }}
        />
      ))}
      {state.loading < 100 && !state.error && (
        <div className="wave-message">Reading waveform… {state.loading}%</div>
      )}
      {state.error && <div className="wave-message is-error">{state.error}</div>}
    </div>
  )
})
