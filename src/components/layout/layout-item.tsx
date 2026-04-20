import type {
  DraggableAttributes,
  DraggableSyntheticListeners,
} from '@dnd-kit/core'
import {
  alpha,
  ListItem,
  ListItemButton,
  ListItemIcon,
  ListItemText,
} from '@mui/material'
import type { CSSProperties, ReactNode } from 'react'
import { useMatch, useNavigate, useResolvedPath } from 'react-router'

import { useVerge } from '@/hooks/use-verge'

interface SortableProps {
  setNodeRef?: (element: HTMLElement | null) => void
  attributes?: DraggableAttributes
  listeners?: DraggableSyntheticListeners
  style?: CSSProperties
  isDragging?: boolean
  disabled?: boolean
}

interface Props {
  to: string
  children: string
  icon: ReactNode[]
  sortable?: SortableProps
}
export const LayoutItem = (props: Props) => {
  const { to, children, icon, sortable } = props
  const { verge } = useVerge()
  const { menu_icon } = verge ?? {}
  const navCollapsed = verge?.collapse_navbar ?? false
  const resolved = useResolvedPath(to)
  const match = useMatch({ path: resolved.pathname, end: true })
  const navigate = useNavigate()

  const effectiveMenuIcon =
    navCollapsed && menu_icon === 'disable' ? 'monochrome' : menu_icon

  const { setNodeRef, attributes, listeners, style, isDragging, disabled } =
    sortable ?? {}

  const draggable = Boolean(sortable) && !disabled
  const dragHandleProps = draggable
    ? { ...(attributes ?? {}), ...(listeners ?? {}) }
    : undefined

  return (
    <ListItem
      ref={setNodeRef}
      style={style}
      sx={[
        { py: 0.5, maxWidth: 250, mx: 'auto', padding: '4px 0px' },
        isDragging ? { opacity: 0.78 } : {},
      ]}
    >
      <ListItemButton
        selected={!!match}
        {...(dragHandleProps ?? {})}
        sx={[
          {
            borderRadius: '22px',
            marginLeft: 0,
            paddingLeft: 1.25,
            paddingRight: 1.25,
            marginRight: 0,
            cursor: draggable ? 'grab' : 'pointer',
            minHeight: 56,
            background: 'var(--surface-panel)',
            border: '1px solid var(--surface-border)',
            boxShadow: 'var(--shadow-raised-sm)',
            '&:active': draggable ? { cursor: 'grabbing' } : {},
            '& .MuiListItemText-primary': {
              color: 'text.primary',
              fontWeight: '700',
            },
            '&:hover': {
              background: 'var(--surface-panel-inset)',
              boxShadow: 'var(--shadow-inset)',
            },
          },
          ({ palette: { mode, primary } }) => {
            const selectedBackground =
              mode === 'light'
                ? `linear-gradient(135deg, ${alpha(primary.main, 0.18)}, rgba(255,255,255,0.88))`
                : `linear-gradient(135deg, ${alpha(primary.main, 0.2)}, rgba(255,255,255,0.04))`
            const color = mode === 'light' ? '#162033' : '#f3f6ff'
            return {
              '&.Mui-selected': {
                background: selectedBackground,
                boxShadow: 'var(--shadow-inset)',
              },
              '&.Mui-selected:hover': {
                background: selectedBackground,
              },
              '&.Mui-selected .MuiListItemText-primary': { color },
            }
          },
        ]}
        title={navCollapsed ? children : undefined}
        aria-label={navCollapsed ? children : undefined}
        onClick={() => navigate(to)}
      >
        {(effectiveMenuIcon === 'monochrome' || !effectiveMenuIcon) && (
          <ListItemIcon
            sx={{
              color: 'text.primary',
              marginLeft: '8px',
              cursor: draggable ? 'grab' : 'inherit',
            }}
          >
            {icon[0]}
          </ListItemIcon>
        )}
        {effectiveMenuIcon === 'colorful' && (
          <ListItemIcon sx={{ cursor: draggable ? 'grab' : 'inherit' }}>
            {icon[1]}
          </ListItemIcon>
        )}
        <ListItemText
          sx={{
            textAlign: 'center',
            marginLeft: effectiveMenuIcon === 'disable' ? '' : '-30px',
          }}
          primary={children}
        />
      </ListItemButton>
    </ListItem>
  )
}
