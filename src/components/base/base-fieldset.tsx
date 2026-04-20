import { Box, styled } from '@mui/material'
import type React from 'react'

type Props = {
  label: string
  fontSize?: string
  width?: string
  padding?: string
  children?: React.ReactNode
}

export const BaseFieldset: React.FC<Props> = ({
  label,
  fontSize,
  width,
  padding,
  children,
}: Props) => {
  const Fieldset = styled(Box)<{ component?: string }>(() => ({
    position: 'relative',
    border: '1px solid var(--surface-border)',
    borderRadius: '24px',
    background: 'var(--surface-panel)',
    boxShadow: 'var(--shadow-inset)',
    width: width ?? 'auto',
    padding: padding ?? '18px',
  }))

  const Label = styled('legend')(({ theme }) => ({
    position: 'absolute',
    top: '-12px',
    left: padding ?? '18px',
    padding: '0 10px',
    borderRadius: '999px',
    background: 'var(--surface-panel)',
    color: theme.palette.text.primary,
    fontSize: fontSize ?? '1em',
    fontWeight: 700,
  }))

  return (
    <Fieldset component="fieldset">
      <Label>{label}</Label>
      {children}
    </Fieldset>
  )
}
