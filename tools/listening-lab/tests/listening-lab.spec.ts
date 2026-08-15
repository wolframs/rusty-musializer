import { expect, test } from '@playwright/test'
import { resolve } from 'node:path'

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Listening Lab E2E fixture' })).toBeVisible()
  await expect(page.getByText('Reading waveform…')).toBeHidden()
})

test('loads waveforms and exposes precise playback controls', async ({ page }) => {
  await expect(page.getByTestId('waveform-A')).toBeVisible()
  await expect(page.getByTestId('waveform-B')).toBeVisible()
  await expect(page.getByTestId('playhead')).toHaveText('00:01.000')

  const seek = page.getByLabel('Seek time in seconds or mm:ss.mmm')
  await seek.fill('00:01.250')
  await seek.press('Enter')
  await expect(page.getByTestId('playhead')).toHaveText('00:01.250')

  await page.getByRole('button', { name: /B Track B/ }).click()
  await expect(page.getByRole('button', { name: /B Track B/ })).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('playhead')).toHaveText('00:01.250')

  await page.getByRole('button', { name: '+10 ms' }).click()
  await expect(page.getByTestId('playhead')).toHaveText('00:01.260')
  await page.screenshot({
    path: '../../build/listening-lab-e2e/listening-lab.png',
    fullPage: true,
  })
})

test('saves answer revisions and restores progress after reload', async ({ page }) => {
  await page.getByRole('group', { name: 'Answer choices' }).getByRole('button', { name: '2 B' }).click()
  await expect(page.getByText('Saved; 1 required choice remains')).toBeVisible()
  await expect(page.getByText('0/2 answered')).toBeVisible()

  await page.getByRole('group', { name: 'How clear is the preference?' })
    .getByRole('button', { name: /Immediately clear/ }).click()
  await expect(page.getByText('Saved to the append-only answer log')).toBeVisible()
  await expect(page.getByText('1/2 answered')).toBeVisible()
  await page.getByRole('group', { name: 'What drove the choice?' })
    .getByRole('button', { name: 'Timing' }).click()
  await page.getByRole('button', { name: 'Capture 00:01.000' }).click()

  await page.screenshot({
    path: '../../build/listening-lab-e2e/structured-feedback.png',
    fullPage: true,
  })

  await page.getByRole('button', { name: 'Next' }).click()
  await page.getByLabel('Answer').fill('Candidate B has a higher tone at 00:02.000.')
  await page.getByLabel('Notes and timestamps').fill('Reviewed at normal speed.')
  await page.getByRole('button', { name: 'Save feedback' }).click()
  await expect(page.getByText('Saved to the append-only answer log')).toBeVisible()
  await expect(page.getByText('2/2 answered')).toBeVisible()

  await page.reload()
  await expect(page.getByText('2/2 answered')).toBeVisible()
  await page.getByRole('button', { name: /01.*Answered/ }).click()
  await expect(page.getByRole('group', { name: 'How clear is the preference?' })
    .getByRole('button', { name: /Immediately clear/ })).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByRole('button', { name: 'Timing' })).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByRole('button', { name: /Remove captured time 00:01.000/ })).toBeVisible()
  await page.getByRole('button', { name: /02.*Answered/ }).click()
  await expect(page.getByLabel('Answer')).toHaveValue('Candidate B has a higher tone at 00:02.000.')
  await expect(page.getByLabel('Notes and timestamps')).toHaveValue('Reviewed at normal speed.')
})

test('supports an external visual runner without exposing a duplicate player', async ({ page }) => {
  await page.getByLabel('Session').selectOption('external-e2e')
  await expect(page.getByRole('heading', { name: 'External companion E2E fixture' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Muted visual runner' })).toBeVisible()
  await expect(page.getByText('cargo run -- --mute --protocol build/example.protocol.json')).toBeVisible()
  await expect(page.getByTestId('waveform-A')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Play' })).toHaveCount(0)

  await page.getByRole('group', { name: 'Answer choices' }).getByRole('button', { name: '2 fix' }).click()
  await expect(page.getByText('Saved; 1 required choice remains')).toBeVisible()
  await expect(page.getByText('0/1 answered')).toBeVisible()
  await page.getByRole('group', { name: 'What needs repair?' }).getByRole('button', { name: 'Motion' }).click()
  await expect(page.getByText('Saved to the append-only answer log')).toBeVisible()
  await expect(page.getByText('1/1 answered')).toBeVisible()
})

test('keeps blind sources private and serves seekable byte ranges', async ({ request }) => {
  const protocol = await request.get('/api/protocol?protocol=e2e')
  expect(protocol.ok()).toBeTruthy()
  const body = await protocol.text()
  expect(body).not.toContain('candidate-a')
  expect(body).not.toContain('candidate-b')
  expect(body).not.toContain('.wav')

  const rawPath = resolve(
    new URL('.', import.meta.url).pathname,
    '../../../build/listening-lab-e2e/protocols/e2e.listen.json',
  )
  const rawProtocol = await request.get(`/@fs/${encodeURI(rawPath)}`)
  expect(rawProtocol.ok()).toBeFalsy()
  expect(await rawProtocol.text()).not.toContain('candidate-a')
  const bundledProtocol = await request.get('/protocols/example-ab.listen.json')
  expect(bundledProtocol.ok()).toBeFalsy()
  expect(await bundledProtocol.text()).not.toContain('../../../build/fixture.wav')

  const audio = await request.get('/api/audio?protocol=e2e&track=A', {
    headers: { Range: 'bytes=0-9' },
  })
  expect(audio.status()).toBe(206)
  expect((await audio.body()).byteLength).toBe(10)
  expect(audio.headers()['content-range']).toMatch(/^bytes 0-9\//)
})
