import { describe, expect, it } from 'vitest'
import { clampTime, formatTime } from './time'

describe('time formatting', () => {
  it('keeps millisecond precision and minute carry', () => {
    expect(formatTime(65.4321)).toBe('01:05.432')
    expect(formatTime(0)).toBe('00:00.000')
  })

  it('clamps precise seeks to the media bounds', () => {
    expect(clampTime(-0.001, 20)).toBe(0)
    expect(clampTime(20.001, 20)).toBe(20)
    expect(clampTime(4.125, 20)).toBe(4.125)
  })
})
