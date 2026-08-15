import { describe, expect, it } from 'vitest'
import {
  feedbackFieldVisible,
  pruneHiddenResponses,
  requiredFeedbackRemaining,
} from './FeedbackFields'
import type { AnswerDraft, FeedbackForm } from './types'

const form: FeedbackForm = {
  fields: [
    {
      id: 'fit',
      type: 'scale',
      label: 'Fit',
      required: true,
      options: [
        { value: 'weak', label: 'Weak' },
        { value: 'strong', label: 'Strong' },
      ],
      show_when: { field: 'answer', any_of: ['keep', 'fix'] },
    },
    {
      id: 'repairs',
      type: 'multi',
      label: 'Repairs',
      required: true,
      options: [
        { value: 'motion', label: 'Motion' },
        { value: 'palette', label: 'Palette' },
      ],
      show_when: { field: 'answer', any_of: ['fix'] },
    },
  ],
}

describe('conditional structured feedback', () => {
  it('counts only visible required fields', () => {
    expect(requiredFeedbackRemaining(form, { answer: 'keep', note: '', responses: {} })).toBe(1)
    expect(requiredFeedbackRemaining(form, { answer: 'fix', note: '', responses: {} })).toBe(2)
    expect(requiredFeedbackRemaining(form, {
      answer: 'fix',
      note: '',
      responses: { fit: 'strong', repairs: ['motion'] },
    })).toBe(0)
  })

  it('drops answers that a changed verdict makes irrelevant', () => {
    const draft: AnswerDraft = {
      answer: 'keep',
      note: '',
      responses: { fit: 'strong', repairs: ['motion'] },
    }
    const next = pruneHiddenResponses(form, draft)
    expect(next.responses).toEqual({ fit: 'strong' })
    expect(feedbackFieldVisible(form.fields[1], draft)).toBe(false)
  })
})
