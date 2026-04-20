import { TextField, type TextFieldProps, styled } from '@mui/material'
import { useTranslation } from 'react-i18next'

export const BaseStyledTextField = styled((props: TextFieldProps) => {
  const { t } = useTranslation()

  return (
    <TextField
      autoComplete="new-password"
      hiddenLabel
      fullWidth
      size="small"
      variant="outlined"
      spellCheck="false"
      placeholder={t('shared.placeholders.filter')}
      sx={{ input: { py: 0.8, px: 1.35 } }}
      {...props}
    />
  )
})(() => ({
  '& .MuiInputBase-root': {
    background: 'var(--surface-panel)',
    boxShadow: 'var(--shadow-inset)',
  },
}))
