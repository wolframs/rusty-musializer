#!/usr/bin/env node
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { basename, resolve } from 'node:path'

const inputs = process.argv.slice(2)
if (!inputs.length) {
  console.error('usage: npm run import:cx4 -- PATH.protocol.json [PATH.protocol.json ...]')
  process.exit(2)
}

const verdicts = ['keep', 'interesting but needs fixing', 'reject']
const pairVerdicts = ['the first look', 'the second look', 'either', 'neither']
const when = (anyOf) => ({ field: 'answer', any_of: anyOf })
const option = (value, label, description) => ({ value, label, ...(description ? { description } : {}) })

const feedbackTemplates = {
  'cx4-single': {
    fields: [
      {
        id: 'track_fit',
        type: 'scale',
        label: 'How naturally does the look fit this music?',
        help: 'Judge the relationship, not whether either the song or scene is good by itself.',
        required: true,
        show_when: when(verdicts),
        options: [
          option('fights', 'Fights it', 'The visual energy contradicts the music.'),
          option('loose', 'Loosely related', 'Plausible, but it could belong to many tracks.'),
          option('works', 'Works with it', 'The major visual choices support the passage.'),
          option('authored', 'Feels authored', 'It looks deliberately made for this passage.'),
        ],
      },
      {
        id: 'strengths',
        type: 'multi',
        label: 'What earns the result its value?',
        help: 'Choose up to three. These tell the agent what not to erase while tuning.',
        max_selections: 3,
        show_when: when(['keep', 'interesting but needs fixing']),
        options: [
          option('music-fit', 'Fits the music'),
          option('composition', 'Strong composition'),
          option('motion', 'Motion feels alive'),
          option('palette', 'Palette works'),
          option('depth', 'Good depth / layering'),
          option('distinctive', 'Distinctive identity'),
        ],
      },
      {
        id: 'repairs',
        type: 'multi',
        label: 'What most needs attention?',
        help: 'Choose one to three concrete failure modes.',
        required: true,
        max_selections: 3,
        show_when: when(['interesting but needs fixing', 'reject']),
        options: [
          option('too-busy', 'Too busy'),
          option('too-static', 'Too static'),
          option('weak-focus', 'Weak focal point'),
          option('palette-mismatch', 'Palette misses the track'),
          option('motion-mismatch', 'Motion misses the rhythm'),
          option('generic', 'Feels generic'),
          option('illegible', 'Hard to read visually'),
        ],
      },
      {
        id: 'repair_distance',
        type: 'scale',
        label: 'How far is it from keepable?',
        required: true,
        show_when: when(['interesting but needs fixing']),
        options: [
          option('small', 'One small tweak'),
          option('focused', 'A focused tuning pass'),
          option('large', 'Most of the look needs rebuilding'),
        ],
      },
    ],
    note: {
      collapsed: true,
      label: 'Add a note only if the controls missed something',
      placeholder: 'Name the visual moment or change the controls could not express.',
    },
  },
  'cx4-pair': {
    fields: [
      {
        id: 'difference',
        type: 'scale',
        label: 'How visually distinct were the two looks?',
        help: 'This directly checks whether consecutive Surprise presses produce meaningfully different results.',
        required: true,
        show_when: when(pairVerdicts),
        options: [
          option('barely', 'Barely different'),
          option('noticeable', 'Noticeably different'),
          option('clear', 'Clearly different'),
          option('direction', 'Different directions'),
        ],
      },
      {
        id: 'separators',
        type: 'multi',
        label: 'What separated them?',
        help: 'Choose up to three dimensions that actually changed the judgment.',
        max_selections: 3,
        show_when: when(pairVerdicts),
        options: [
          option('music-fit', 'Fit to the music'),
          option('composition', 'Composition'),
          option('motion', 'Motion'),
          option('palette', 'Palette'),
          option('intensity', 'Intensity'),
          option('identity', 'Distinctiveness'),
        ],
      },
      {
        id: 'neither_problem',
        type: 'multi',
        label: 'Why was neither keepable?',
        required: true,
        max_selections: 2,
        show_when: when(['neither']),
        options: [
          option('busy', 'Both too busy'),
          option('static', 'Both too static'),
          option('generic', 'Both feel generic'),
          option('track-mismatch', 'Both miss the track'),
          option('different-flaws', 'Different flaws'),
        ],
      },
      {
        id: 'confidence',
        type: 'single',
        label: 'How clear was your preference?',
        required: true,
        show_when: when(pairVerdicts),
        options: [
          option('guess', 'Close to a guess'),
          option('lean', 'A real but slight preference'),
          option('clear', 'Immediately clear'),
        ],
      },
    ],
    note: {
      collapsed: true,
      label: 'Add a note only if the controls missed something',
      placeholder: 'Describe the decisive difference the choices did not capture.',
    },
  },
}

const outputDirectory = resolve(new URL('../protocols', import.meta.url).pathname)
await mkdir(outputDirectory, { recursive: true })

for (const input of inputs) {
  const inputPath = resolve(input)
  const legacy = JSON.parse(await readFile(inputPath, 'utf8'))
  if (legacy.schema !== 'musializer.protocol/v1') {
    throw new Error(`${input}: expected musializer.protocol/v1`)
  }
  const sourceName = basename(inputPath)
  const stem = sourceName.replace(/\.protocol\.json$/i, '')
  const id = `${stem}-feedback`
  const output = {
    schema: 'musializer.listening-test/v1',
    id,
    title: `${legacy.title} — feedback sheet`,
    instructions: 'Keep the Rust visual runner and this sheet on the same question number. Judge the look in Rust, answer here, then press N in Rust to advance. Do not open the .key.json until both sessions are complete.',
    blind: true,
    playback: 'external',
    companion: {
      label: 'Rust visual protocol runner',
      command: `cargo run -- --protocol build/protocols/${sourceName}`,
      help: 'The Rust app owns audio, scene state, and blind A/B order. This browser owns only the structured feedback log.',
    },
    tracks: [
      {
        id: 'reference-audio',
        label: basename(legacy.audio.path),
        path: legacy.audio.path,
      },
    ],
    feedback_templates: feedbackTemplates,
    questions: legacy.items.map((item) => ({
      id: item.id,
      at_seconds: item.at_seconds,
      window: item.window,
      question: item.question,
      detail: item.apply?.snapshots?.b
        ? 'Use B in the Rust runner to alternate the two blind looks. “First” and “second” refer to the order the runner presents.'
        : 'Judge the complete look first. The next controls capture why it is keepable or what would need to change.',
      kind: item.kind,
      options: item.options,
      tracks: ['reference-audio'],
      required: true,
      feedback: item.apply?.snapshots?.b ? 'cx4-pair' : 'cx4-single',
    })),
  }
  const outputPath = resolve(outputDirectory, `${id}.listen.json`)
  await writeFile(outputPath, `${JSON.stringify(output, null, 2)}\n`)
  console.log(`wrote ${outputPath}`)
}
