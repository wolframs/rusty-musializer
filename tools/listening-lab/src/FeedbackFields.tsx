import { formatTime } from './time'
import type {
  AnswerDraft,
  FeedbackField,
  FeedbackForm,
  FeedbackValue,
} from './types'

export function feedbackFieldVisible(field: FeedbackField, draft: AnswerDraft): boolean {
  if (!field.show_when) return true
  const value = field.show_when.field === 'answer'
    ? draft.answer
    : draft.responses[field.show_when.field]
  if (Array.isArray(value)) {
    return value.some((item) => field.show_when!.any_of.includes(String(item)))
  }
  return field.show_when.any_of.includes(String(value))
}

export function pruneHiddenResponses(form: FeedbackForm | undefined, draft: AnswerDraft): AnswerDraft {
  if (!form) return draft
  const responses: Record<string, FeedbackValue> = {}
  for (const field of form.fields) {
    if (feedbackFieldVisible(field, { ...draft, responses })) {
      const value = draft.responses[field.id]
      if (value !== undefined) responses[field.id] = value
    }
  }
  return { ...draft, responses }
}

export function requiredFeedbackRemaining(form: FeedbackForm | undefined, draft: AnswerDraft): number {
  return (form?.fields || []).filter((field) => {
    if (!field.required || !feedbackFieldVisible(field, draft)) return false
    const value = draft.responses[field.id]
    return value === undefined || value === '' || (Array.isArray(value) && value.length === 0)
  }).length
}

interface Props {
  form: FeedbackForm
  draft: AnswerDraft
  playhead: number
  onCommit: (draft: AnswerDraft) => void
}

export function FeedbackFields({ form, draft, playhead, onCommit }: Props) {
  const visible = form.fields.filter((field) => feedbackFieldVisible(field, draft))
  if (!visible.length) return null

  const commit = (field: FeedbackField, value: FeedbackValue) => {
    onCommit(pruneHiddenResponses(form, {
      ...draft,
      responses: { ...draft.responses, [field.id]: value },
    }))
  }

  return (
    <div className="structured-feedback">
      {visible.map((field) => {
        const value = draft.responses[field.id]
        return (
          <section className="feedback-field" key={field.id} aria-labelledby={`field-${field.id}`}>
            <header>
              <div>
                <h3 id={`field-${field.id}`}>{field.label}</h3>
                {field.help && <p>{field.help}</p>}
              </div>
              {field.required && <span>Required</span>}
            </header>

            {(field.type === 'single' || field.type === 'scale') && (
              <div
                className={field.type === 'scale' ? 'scale-options' : 'field-options'}
                role="group"
                aria-label={field.label}
              >
                {field.options?.map((option) => (
                  <button
                    type="button"
                    key={option.value}
                    className={value === option.value ? 'is-selected' : ''}
                    aria-pressed={value === option.value}
                    onClick={() => commit(field, option.value)}
                  >
                    <strong>{option.label}</strong>
                    {option.description && <small>{option.description}</small>}
                  </button>
                ))}
              </div>
            )}

            {field.type === 'multi' && (
              <div className="choice-chips" role="group" aria-label={field.label}>
                {field.options?.map((option) => {
                  const current = Array.isArray(value) ? value as string[] : []
                  const selected = current.includes(option.value)
                  const count = current.length
                  const atLimit = count >= (field.max_selections || field.options!.length)
                  return (
                    <button
                      type="button"
                      key={option.value}
                      className={selected ? 'is-selected' : ''}
                      aria-pressed={selected}
                      disabled={!selected && atLimit}
                      title={option.description}
                      onClick={() => {
                        commit(field, selected
                          ? current.filter((item) => item !== option.value)
                          : [...current, option.value])
                      }}
                    >
                      {option.label}
                    </button>
                  )
                })}
              </div>
            )}

            {field.type === 'timestamps' && (
              <div className="timestamp-field">
                <button
                  type="button"
                  className="capture-time"
                  disabled={Array.isArray(value) && value.length >= (field.max_selections || 4)}
                  onClick={() => {
                    const current = Array.isArray(value) ? value as number[] : []
                    commit(field, [...current, playhead])
                  }}
                >
                  Capture {formatTime(playhead)}
                </button>
                <div className="captured-times" aria-live="polite">
                  {(Array.isArray(value) ? value as number[] : []).map((time, index) => (
                    <button
                      type="button"
                      key={`${time}-${index}`}
                      aria-label={`Remove captured time ${formatTime(time)}`}
                      onClick={() => commit(
                        field,
                        (value as number[]).filter((_item, itemIndex) => itemIndex !== index),
                      )}
                    >
                      {formatTime(time)} <span>Remove</span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </section>
        )
      })}
    </div>
  )
}
