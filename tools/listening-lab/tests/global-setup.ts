import { mkdir, rm, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'

function wav(durationSeconds: number, frequency: number): Buffer {
  const sampleRate = 16_000
  const samples = sampleRate * durationSeconds
  const dataBytes = samples * 2
  const output = Buffer.alloc(44 + dataBytes)
  output.write('RIFF', 0)
  output.writeUInt32LE(36 + dataBytes, 4)
  output.write('WAVE', 8)
  output.write('fmt ', 12)
  output.writeUInt32LE(16, 16)
  output.writeUInt16LE(1, 20)
  output.writeUInt16LE(1, 22)
  output.writeUInt32LE(sampleRate, 24)
  output.writeUInt32LE(sampleRate * 2, 28)
  output.writeUInt16LE(2, 32)
  output.writeUInt16LE(16, 34)
  output.write('data', 36)
  output.writeUInt32LE(dataBytes, 40)
  for (let index = 0; index < samples; index += 1) {
    const envelope = index % 4_000 < 180 ? 1 : 0.22
    const sample = Math.round(Math.sin((index * frequency * Math.PI * 2) / sampleRate) * 20_000 * envelope)
    output.writeInt16LE(sample, 44 + index * 2)
  }
  return output
}

export default async function setup() {
  const repoRoot = resolve(new URL('.', import.meta.url).pathname, '../../..')
  const testRoot = resolve(repoRoot, 'build/listening-lab-e2e')
  const protocols = resolve(testRoot, 'protocols')
  await rm(testRoot, { recursive: true, force: true })
  await mkdir(protocols, { recursive: true })
  await writeFile(resolve(testRoot, 'candidate-a.wav'), wav(3, 220))
  await writeFile(resolve(testRoot, 'candidate-b.wav'), wav(3, 330))
  await writeFile(
    resolve(protocols, 'e2e.listen.json'),
    JSON.stringify(
      {
        schema: 'musializer.listening-test/v1',
        id: 'e2e',
        title: 'Listening Lab E2E fixture',
        instructions: 'Generated locally for the headless test.',
        blind: true,
        tracks: [
          { id: 'candidate-a', label: 'Candidate A source', path: '../candidate-a.wav' },
          { id: 'candidate-b', label: 'Candidate B source', path: '../candidate-b.wav' },
        ],
        questions: [
          {
            id: 'onset',
            at_seconds: 1.25,
            window: { pre: 0.25, post: 0.75 },
            question: 'Which candidate has the clearer onset?',
            kind: 'choice',
            options: ['A', 'B', 'either', 'neither'],
            tracks: ['candidate-a', 'candidate-b'],
            required: true,
            feedback: {
              fields: [
                {
                  id: 'confidence',
                  type: 'scale',
                  label: 'How clear is the preference?',
                  required: true,
                  options: [
                    { value: 'close', label: 'Close call' },
                    { value: 'clear', label: 'Immediately clear' },
                  ],
                },
                {
                  id: 'evidence',
                  type: 'multi',
                  label: 'What drove the choice?',
                  max_selections: 2,
                  options: [
                    { value: 'timing', label: 'Timing' },
                    { value: 'clarity', label: 'Clarity' },
                    { value: 'tone', label: 'Tone' },
                  ],
                },
                {
                  id: 'moments',
                  type: 'timestamps',
                  label: 'Capture decisive moments',
                  max_selections: 2,
                },
              ],
              note: {
                collapsed: true,
                label: 'Add context the controls missed',
              },
            },
          },
          {
            id: 'detail',
            at_seconds: 2.0,
            window: { pre: 0.5, post: 0.5 },
            question: 'Describe the audible difference.',
            kind: 'text',
            tracks: ['candidate-a', 'candidate-b'],
            required: true,
          },
        ],
      },
      null,
      2,
    ),
  )
  await writeFile(
    resolve(protocols, 'external-e2e.listen.json'),
    JSON.stringify(
      {
        schema: 'musializer.listening-test/v1',
        id: 'external-e2e',
        title: 'External companion E2E fixture',
        instructions: 'Keep both tools on the same question.',
        blind: true,
        playback: 'external',
        companion: {
          label: 'Muted visual runner',
          command: 'cargo run -- --mute --protocol build/example.protocol.json',
          help: 'The companion owns playback; this page records the judgment.',
        },
        tracks: [
          { id: 'reference', label: 'Reference', path: '../candidate-a.wav' },
        ],
        questions: [
          {
            id: 'look',
            at_seconds: 1.5,
            window: { pre: 0.5, post: 1.0 },
            question: 'Would you keep this look?',
            kind: 'choice',
            options: ['keep', 'fix', 'reject'],
            tracks: ['reference'],
            required: true,
            feedback: {
              fields: [
                {
                  id: 'repair',
                  type: 'multi',
                  label: 'What needs repair?',
                  required: true,
                  show_when: { field: 'answer', any_of: ['fix', 'reject'] },
                  options: [
                    { value: 'motion', label: 'Motion' },
                    { value: 'palette', label: 'Palette' },
                  ],
                },
              ],
            },
          },
        ],
      },
      null,
      2,
    ),
  )
}
