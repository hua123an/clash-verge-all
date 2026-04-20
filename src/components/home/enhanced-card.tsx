import { Box, Typography, alpha, useTheme } from '@mui/material'
import { forwardRef, type ReactNode } from 'react'

// 自定义卡片组件接口
interface EnhancedCardProps {
  title: ReactNode
  icon: ReactNode
  action?: ReactNode
  children: ReactNode
  iconColor?: 'primary' | 'secondary' | 'error' | 'warning' | 'info' | 'success'
  minHeight?: number | string
  noContentPadding?: boolean
}

// 自定义卡片组件
export const EnhancedCard = forwardRef<HTMLElement, EnhancedCardProps>(
  (
    {
      title,
      icon,
      action,
      children,
      iconColor = 'primary',
      minHeight,
      noContentPadding = false,
    },
    ref,
  ) => {
    const theme = useTheme()
    const isDark = theme.palette.mode === 'dark'

    // 统一的标题截断样式
    const titleTruncateStyle = {
      minWidth: 0,
      maxWidth: '100%',
      overflow: 'hidden',
      textOverflow: 'ellipsis',
      whiteSpace: 'nowrap',
      display: 'block',
    }

    return (
      <Box
        sx={{
          height: '100%',
          display: 'flex',
          flexDirection: 'column',
          borderRadius: '30px',
          background: 'var(--surface-panel)',
          border: '1px solid var(--surface-border)',
          boxShadow: 'var(--shadow-raised)',
          overflow: 'hidden',
        }}
        ref={ref}
      >
        <Box
          sx={{
            px: 2.5,
            py: 1.5,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            borderBottom: 1,
            borderColor: alpha(theme.palette.divider, isDark ? 0.55 : 0.8),
            background: isDark
              ? 'linear-gradient(180deg, rgba(255,255,255,0.03), rgba(255,255,255,0))'
              : 'linear-gradient(180deg, rgba(255,255,255,0.72), rgba(255,255,255,0))',
          }}
        >
          <Box
            sx={{
              display: 'flex',
              alignItems: 'center',
              minWidth: 0,
              flex: 1,
              overflow: 'hidden',
            }}
          >
            <Box
              sx={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                borderRadius: '18px',
                width: 42,
                height: 42,
                mr: 1.75,
                flexShrink: 0,
                background: `linear-gradient(145deg, ${alpha(
                  theme.palette[iconColor].main,
                  0.18,
                )}, ${alpha(theme.palette[iconColor].main, 0.08)})`,
                color: theme.palette[iconColor].main,
                boxShadow: 'var(--shadow-inset)',
              }}
            >
              {icon}
            </Box>
            <Box sx={{ minWidth: 0, flex: 1 }}>
              {typeof title === 'string' ? (
                <Typography
                  variant="h6"
                  sx={{
                    ...titleTruncateStyle,
                    fontWeight: 'medium',
                    fontSize: 18,
                  }}
                  title={title}
                >
                  {title}
                </Typography>
              ) : (
                <Box sx={titleTruncateStyle}>{title}</Box>
              )}
            </Box>
          </Box>
          {action && <Box sx={{ ml: 2, flexShrink: 0 }}>{action}</Box>}
        </Box>
        <Box
          sx={{
            flex: 1,
            display: 'flex',
            flexDirection: 'column',
            p: noContentPadding ? 0 : 2.5,
            ...(minHeight && { minHeight }),
          }}
        >
          {children}
        </Box>
      </Box>
    )
  },
)
