import { ChevronRightRounded } from '@mui/icons-material'
import {
  Box,
  List,
  ListItem,
  ListItemButton,
  ListItemText,
  ListSubheader,
} from '@mui/material'
import CircularProgress from '@mui/material/CircularProgress'
import React, { ReactNode, useState } from 'react'

import isAsyncFunction from '@/utils/is-async-function'

interface ItemProps {
  label: ReactNode
  extra?: ReactNode
  children?: ReactNode
  secondary?: ReactNode
  onClick?: () => void | Promise<any>
}

export const SettingItem: React.FC<ItemProps> = ({
  label,
  extra,
  children,
  secondary,
  onClick,
}) => {
  const clickable = !!onClick

  const primary = (
    <Box sx={{ display: 'flex', alignItems: 'center', fontSize: '14px' }}>
      <span>{label}</span>
      {extra ? extra : null}
    </Box>
  )

  const [isLoading, setIsLoading] = useState(false)
  const handleClick = () => {
    if (onClick) {
      if (isAsyncFunction(onClick)) {
        setIsLoading(true)
        onClick()!.finally(() => setIsLoading(false))
      } else {
        onClick()
      }
    }
  }

  return clickable ? (
    <ListItem disablePadding sx={{ mb: 1 }}>
      <ListItemButton
        onClick={handleClick}
        disabled={isLoading}
        sx={{
          minHeight: 58,
          px: 1.5,
          borderRadius: '22px',
          background: 'var(--surface-panel)',
          border: '1px solid var(--surface-border)',
          boxShadow: 'var(--shadow-raised-sm)',
        }}
      >
        <ListItemText primary={primary} secondary={secondary} />
        {isLoading ? (
          <CircularProgress color="inherit" size={20} />
        ) : (
          <ChevronRightRounded />
        )}
      </ListItemButton>
    </ListItem>
  ) : (
    <ListItem
      sx={{
        pt: 1,
        pb: 1,
        px: 1.5,
        mb: 1,
        borderRadius: '22px',
        background: 'var(--surface-panel)',
        border: '1px solid var(--surface-border)',
        boxShadow: 'var(--shadow-raised-sm)',
      }}
    >
      <ListItemText primary={primary} secondary={secondary} />
      {children}
    </ListItem>
  )
}

export const SettingList: React.FC<{
  title: string
  children: ReactNode
}> = ({ title, children }) => (
  <List
    sx={{
      p: 1.5,
      borderRadius: '28px',
      background: 'var(--surface-panel)',
      border: '1px solid var(--surface-border)',
      boxShadow: 'var(--shadow-raised)',
    }}
  >
    <ListSubheader
      sx={[
        {
          background: 'transparent',
          fontSize: '16px',
          fontWeight: '700',
          px: 1,
          pb: 1,
          mb: 1,
        },
        ({ palette }) => {
          return {
            color: palette.text.primary,
          }
        },
      ]}
      disableSticky
    >
      {title}
    </ListSubheader>

    {children}
  </List>
)
