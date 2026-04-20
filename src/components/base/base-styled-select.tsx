import { Select, styled, type SelectProps } from '@mui/material'

export const BaseStyledSelect = styled((props: SelectProps<string>) => {
  return (
    <Select
      size="small"
      autoComplete="new-password"
      sx={{
        width: 120,
        height: 42,
        mr: 1,
        '[role="button"]': { py: 1 },
      }}
      {...props}
    />
  )
})({
  background: 'var(--surface-panel)',
  boxShadow: 'var(--shadow-inset)',
})
