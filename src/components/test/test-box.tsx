import { alpha, Box, styled } from '@mui/material'

export const TestBox = styled(Box)(({ theme, 'aria-selected': selected }) => {
  const { mode, primary, text } = theme.palette
  const key = `${mode}-${!!selected}`

  const backgroundColor =
    mode === 'light' ? alpha(primary.main, 0.05) : alpha(primary.main, 0.08)

  const color = {
    'light-true': text.secondary,
    'light-false': text.secondary,
    'dark-true': alpha(text.secondary, 0.65),
    'dark-false': alpha(text.secondary, 0.65),
  }[key]!

  const h2color = {
    'light-true': primary.main,
    'light-false': text.primary,
    'dark-true': primary.main,
    'dark-false': text.primary,
  }[key]!

  return {
    position: 'relative',
    width: '100%',
    display: 'block',
    cursor: 'pointer',
    textAlign: 'left',
    borderRadius: 24,
    boxShadow: 'var(--shadow-raised-sm)',
    padding: '12px 18px',
    boxSizing: 'border-box',
    background: 'var(--surface-panel)',
    border: '1px solid var(--surface-border)',
    color,
    '& h2': { color: h2color },
    transition: 'background-color 0.3s, box-shadow 0.3s',
    '&:hover': {
      background:
        mode === 'light'
          ? `linear-gradient(145deg, ${alpha(primary.main, 0.1)}, rgba(255,255,255,0.88))`
          : `linear-gradient(145deg, ${alpha(primary.main, 0.14)}, rgba(255,255,255,0.04))`,
      boxShadow: 'var(--shadow-inset)',
    },
  }
})
