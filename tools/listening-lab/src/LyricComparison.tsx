import { formatTime } from './time'
import type { LyricTimingComparison } from './types'

export function LyricComparison({ comparison, time, start, end, onSeek, onReplay }: {
  comparison: LyricTimingComparison
  time: number
  start: number
  end: number
  onSeek: (time: number) => void
  onReplay: () => void
}) {
  const percent = (value: number) => 100 * (value - start) / (end - start)
  const sourceTime = (value: number) => formatTime(value + comparison.source_offset_seconds)
  const gap = comparison.gap
  return <section className="lyric-comparison" aria-label="Timed lyric comparison">
    <header className="lyric-comparison-header">
      <div><h2>Compare the timed lyrics</h2><p>Same audio, same playhead. Which line grouping fits the performance?</p></div>
      <button type="button" onClick={onReplay}>Replay comparison</button>
    </header>
    <p className="lyric-timing-note">Proposed timings · {comparison.timing_note}</p>
    <div className="lyric-comparison-grid">
      {comparison.variants.map((variant) => {
        const active = variant.cues.filter(cue => time >= cue.start_seconds && time < cue.end_seconds)
        return <section key={variant.label} className="lyric-interpretation" aria-label={variant.label}>
          <h3>{variant.label}</h3>
          <div className={`lyric-live ${active.length ? 'is-sung' : ''}`} data-testid={`caption-${variant.cues.length}`}>
            {active.length ? active.map(cue => <div key={cue.start_seconds}>{cue.text}</div>) : <span className="lyric-empty">No line displayed at this playhead</span>}
          </div>
          <div className="lyric-time-axis"><span>{sourceTime(start)}</span><span>Original track</span><span>{sourceTime(end)}</span></div>
          <div className="lyric-cue-lane" aria-label={`${variant.label} timeline`}>
            <div className="lyric-gap" style={{left:`${percent(gap.start_seconds)}%`,width:`${percent(gap.end_seconds)-percent(gap.start_seconds)}%`}} />
            {variant.cues.map((cue, index) => <button type="button" key={cue.start_seconds}
              className={`lyric-cue ${time >= cue.start_seconds && time < cue.end_seconds ? 'is-active' : ''}`}
              style={{left:`${percent(cue.start_seconds)}%`,width:`${percent(cue.end_seconds)-percent(cue.start_seconds)}%`}}
              aria-label={`Seek to ${cue.text}`} onClick={() => onSeek(cue.start_seconds)} title={`${cue.text} · ${sourceTime(cue.start_seconds)}–${sourceTime(cue.end_seconds)}`}>
              {index + 1}
            </button>)}
            {time >= start && time <= end && <div className="lyric-playhead" style={{left:`${percent(time)}%`}} />}
          </div>
          <ol className="lyric-cue-list">{variant.cues.map(cue => <li key={cue.start_seconds}>
            <button type="button" onClick={() => onSeek(cue.start_seconds)}><span>{cue.text}</span><small>{sourceTime(cue.start_seconds)}–{sourceTime(cue.end_seconds)}</small></button>
          </li>)}</ol>
        </section>
      })}
    </div>
    <div className="lyric-gap-description">
      <span>Proposed gap: <strong>{sourceTime(gap.start_seconds)}–{sourceTime(gap.end_seconds)}</strong> in {comparison.source_title} ({(gap.end_seconds-gap.start_seconds).toFixed(2)} s)</span>
      <button type="button" onClick={() => onSeek(Math.max(start, gap.start_seconds - 0.6))}>Seek before gap</button>
    </div>
  </section>
}
