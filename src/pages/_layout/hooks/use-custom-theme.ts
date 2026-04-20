import {
  alpha,
  createTheme,
  type Shadows,
  type Theme as MuiTheme,
} from '@mui/material'
import {
  getCurrentWebviewWindow,
  type WebviewWindow,
} from '@tauri-apps/api/webviewWindow'
import type { Theme as TauriOsTheme } from '@tauri-apps/api/window'
import { useEffect, useMemo } from 'react'

import { useVerge } from '@/hooks/use-verge'
import { defaultDarkTheme, defaultTheme } from '@/pages/_theme'
import { useSetThemeMode, useThemeMode } from '@/services/states'

const CSS_INJECTION_SCOPE_ROOT = '[data-css-injection-root]'
const CSS_INJECTION_SCOPE_LIMIT =
  ':is(.monaco-editor .view-lines, .monaco-editor .view-line, .monaco-editor .margin, .monaco-editor .margin-view-overlays, .monaco-editor .view-overlays, .monaco-editor [class^="mtk"], .monaco-editor [class*=" mtk"])'
const TOP_LEVEL_AT_RULES = [
  '@charset',
  '@import',
  '@namespace',
  '@font-face',
  '@keyframes',
  '@counter-style',
  '@page',
  '@property',
  '@font-feature-values',
  '@color-profile',
]
let cssScopeSupport: boolean | null = null

type SurfaceTokens = {
  canvas: string
  panel: string
  panelSoft: string
  panelInset: string
  shell: string
  border: string
  outline: string
  shadow: string
  shadowSm: string
  shadowInset: string
}

const buildSurfaceTokens = (mode: 'light' | 'dark'): SurfaceTokens => {
  if (mode === 'light') {
    return {
      canvas: '#E7EDF3',
      panel: '#EDF2F8',
      panelSoft:
        'linear-gradient(145deg, rgba(247, 251, 255, 0.96), rgba(219, 228, 238, 0.98))',
      panelInset:
        'linear-gradient(145deg, rgba(221, 229, 238, 0.98), rgba(247, 251, 255, 0.98))',
      shell:
        'radial-gradient(circle at 12% 18%, rgba(255, 255, 255, 0.95), transparent 28%), radial-gradient(circle at 86% 14%, rgba(74, 120, 246, 0.12), transparent 30%), radial-gradient(circle at 82% 82%, rgba(255, 157, 108, 0.10), transparent 28%), #E7EDF3',
      border: 'rgba(128, 145, 168, 0.18)',
      outline: 'rgba(255, 255, 255, 0.72)',
      shadow:
        '16px 16px 36px rgba(163, 177, 198, 0.42), -12px -12px 28px rgba(255, 255, 255, 0.92)',
      shadowSm:
        '10px 10px 22px rgba(163, 177, 198, 0.28), -8px -8px 18px rgba(255, 255, 255, 0.84)',
      shadowInset:
        'inset 6px 6px 16px rgba(163, 177, 198, 0.28), inset -6px -6px 16px rgba(255, 255, 255, 0.84)',
    }
  }

  return {
    canvas: '#1E2430',
    panel: '#252D3B',
    panelSoft:
      'linear-gradient(145deg, rgba(49, 58, 75, 0.98), rgba(27, 33, 44, 0.98))',
    panelInset:
      'linear-gradient(145deg, rgba(24, 29, 39, 0.98), rgba(48, 57, 74, 0.96))',
    shell:
      'radial-gradient(circle at 14% 16%, rgba(122, 151, 255, 0.14), transparent 24%), radial-gradient(circle at 84% 18%, rgba(255, 176, 119, 0.08), transparent 28%), radial-gradient(circle at 78% 82%, rgba(255, 255, 255, 0.04), transparent 18%), #1E2430',
    border: 'rgba(255, 255, 255, 0.06)',
    outline: 'rgba(255, 255, 255, 0.05)',
    shadow:
      '18px 18px 34px rgba(8, 12, 18, 0.56), -10px -10px 24px rgba(67, 78, 103, 0.22)',
    shadowSm:
      '12px 12px 24px rgba(9, 12, 18, 0.4), -8px -8px 18px rgba(63, 74, 98, 0.16)',
    shadowInset:
      'inset 7px 7px 14px rgba(8, 12, 18, 0.55), inset -6px -6px 14px rgba(63, 74, 98, 0.18)',
  }
}

const buildShadows = (surface: SurfaceTokens): Shadows => {
  const shadows = Array(25).fill(surface.shadow) as Shadows
  shadows[0] = 'none'
  shadows[1] = surface.shadowSm
  shadows[2] = surface.shadow
  shadows[3] = surface.shadow
  shadows[4] = surface.shadow
  return shadows
}

const canUseCssScope = () => {
  if (cssScopeSupport !== null) {
    return cssScopeSupport
  }
  try {
    const testStyle = document.createElement('style')
    testStyle.textContent = '@scope (:root) { }'
    document.head.appendChild(testStyle)
    cssScopeSupport = !!testStyle.sheet?.cssRules?.length
    document.head.removeChild(testStyle)
  } catch {
    cssScopeSupport = false
  }
  return cssScopeSupport
}

const wrapCssInjectionWithScope = (css?: string) => {
  if (!css?.trim()) {
    return ''
  }
  const lowerCss = css.toLowerCase()
  const hasTopLevelOnlyRule = TOP_LEVEL_AT_RULES.some((rule) =>
    lowerCss.includes(rule),
  )
  if (hasTopLevelOnlyRule) {
    return null
  }
  const scopeRoot = CSS_INJECTION_SCOPE_ROOT
  const scopeLimit = CSS_INJECTION_SCOPE_LIMIT
  const scopedBlock = `@scope (${scopeRoot}) to (${scopeLimit}) {
${css}
}`
  return scopedBlock
}

/**
 * custom theme
 */
export const useCustomTheme = () => {
  const appWindow: WebviewWindow = useMemo(() => getCurrentWebviewWindow(), [])
  const { verge } = useVerge()
  const { theme_mode, theme_setting } = verge ?? {}
  const mode = useThemeMode()
  const setMode = useSetThemeMode()
  const userBackgroundImage = theme_setting?.background_image || ''
  const hasUserBackground = !!userBackgroundImage

  useEffect(() => {
    if (theme_mode === 'light' || theme_mode === 'dark') {
      setMode(theme_mode)
    }
  }, [theme_mode, setMode])

  useEffect(() => {
    if (theme_mode !== 'system') {
      return
    }

    let isMounted = true

    const timerId = setTimeout(() => {
      if (!isMounted) return
      appWindow
        .theme()
        .then((systemTheme) => {
          if (isMounted && systemTheme) {
            setMode(systemTheme)
          }
        })
        .catch((err) => {
          console.error('Failed to get initial system theme:', err)
        })
    }, 0)

    const unlistenPromise = appWindow.onThemeChanged(({ payload }) => {
      if (isMounted) {
        setMode(payload)
      }
    })

    return () => {
      isMounted = false
      clearTimeout(timerId)
      unlistenPromise
        .then((unlistenFn) => {
          if (typeof unlistenFn === 'function') {
            unlistenFn()
          }
        })
        .catch((err) => {
          console.error('Failed to unlisten from theme changes:', err)
        })
    }
  }, [theme_mode, appWindow, setMode])

  useEffect(() => {
    if (theme_mode === undefined) {
      return
    }

    if (theme_mode === 'system') {
      appWindow.setTheme(null).catch((err) => {
        console.error(
          'Failed to set window theme to follow system (setTheme(null)):',
          err,
        )
      })
    } else if (mode) {
      appWindow.setTheme(mode as TauriOsTheme).catch((err) => {
        console.error(`Failed to set window theme to ${mode}:`, err)
      })
    }
  }, [mode, appWindow, theme_mode])

  const theme = useMemo(() => {
    const setting = theme_setting || {}
    const dt = mode === 'light' ? defaultTheme : defaultDarkTheme
    const surface = buildSurfaceTokens(mode)
    const primaryMain = setting.primary_color || dt.primary_color
    const secondaryMain = setting.secondary_color || dt.secondary_color
    const textPrimary = setting.primary_text || dt.primary_text
    const textSecondary = setting.secondary_text || dt.secondary_text
    let muiTheme: MuiTheme

    try {
      muiTheme = createTheme({
        breakpoints: {
          values: { xs: 0, sm: 650, md: 900, lg: 1200, xl: 1536 },
        },
        palette: {
          mode,
          primary: { main: primaryMain },
          secondary: { main: secondaryMain },
          info: { main: setting.info_color || dt.info_color },
          error: { main: setting.error_color || dt.error_color },
          warning: { main: setting.warning_color || dt.warning_color },
          success: { main: setting.success_color || dt.success_color },
          text: {
            primary: textPrimary,
            secondary: textSecondary,
          },
          background: {
            paper: surface.panel,
            default: surface.canvas,
          },
        },
        shape: {
          borderRadius: 26,
        },
        shadows: buildShadows(surface),
        typography: {
          fontFamily: setting.font_family
            ? `${setting.font_family}, ${dt.font_family}`
            : dt.font_family,
          h6: {
            fontWeight: 700,
            letterSpacing: '-0.02em',
          },
          subtitle1: {
            fontWeight: 700,
          },
          button: {
            fontWeight: 700,
            letterSpacing: '0.01em',
            textTransform: 'none',
          },
        },
        components: {
          MuiPaper: {
            styleOverrides: {
              root: {
                backgroundColor: 'transparent',
                backgroundImage: surface.panelSoft,
                borderRadius: 28,
                border: `1px solid ${surface.border}`,
                boxShadow: surface.shadow,
                backdropFilter: 'blur(22px)',
              },
            },
          },
          MuiDialog: {
            styleOverrides: {
              paper: {
                backgroundImage: surface.panelSoft,
                boxShadow: surface.shadow,
                borderRadius: 30,
              },
            },
          },
          MuiDialogTitle: {
            styleOverrides: {
              root: {
                paddingTop: 22,
                paddingBottom: 10,
                fontWeight: 700,
              },
            },
          },
          MuiDialogContent: {
            styleOverrides: {
              root: {
                paddingTop: 12,
              },
            },
          },
          MuiDialogActions: {
            styleOverrides: {
              root: {
                padding: '12px 20px 20px',
                gap: 10,
              },
            },
          },
          MuiAccordion: {
            styleOverrides: {
              root: {
                backgroundImage: surface.panelSoft,
                borderRadius: 24,
                border: `1px solid ${surface.border}`,
                boxShadow: surface.shadowSm,
                '&::before': {
                  display: 'none',
                },
              },
            },
          },
          MuiAccordionSummary: {
            styleOverrides: {
              root: {
                minHeight: 56,
                paddingInline: 18,
              },
              content: {
                marginBlock: 12,
              },
            },
          },
          MuiAccordionDetails: {
            styleOverrides: {
              root: {
                padding: '0 18px 18px',
              },
            },
          },
          MuiButton: {
            styleOverrides: {
              root: {
                minHeight: 42,
                borderRadius: 18,
                paddingInline: 18,
                boxShadow: surface.shadowSm,
                border: `1px solid ${surface.border}`,
              },
              contained: {
                color: mode === 'light' ? '#FFFFFF' : '#101622',
                backgroundImage: `linear-gradient(135deg, ${alpha(primaryMain, 0.96)}, ${alpha(primaryMain, 0.74)})`,
                '&:hover': {
                  backgroundImage: `linear-gradient(135deg, ${alpha(primaryMain, 1)}, ${alpha(primaryMain, 0.84)})`,
                },
              },
              outlined: {
                color: textPrimary,
                backgroundImage: surface.panelSoft,
              },
              text: {
                backgroundColor: alpha(
                  primaryMain,
                  mode === 'light' ? 0.08 : 0.16,
                ),
              },
            },
          },
          MuiButtonGroup: {
            styleOverrides: {
              root: {
                borderRadius: 20,
                overflow: 'hidden',
                boxShadow: surface.shadowSm,
              },
              grouped: {
                borderColor: surface.border,
              },
            },
          },
          MuiFab: {
            styleOverrides: {
              root: {
                borderRadius: 22,
                border: `1px solid ${surface.border}`,
                backgroundImage: surface.panelSoft,
                boxShadow: surface.shadow,
              },
            },
          },
          MuiChip: {
            styleOverrides: {
              root: {
                borderRadius: 16,
                border: `1px solid ${surface.border}`,
                backgroundImage: surface.panelSoft,
                boxShadow: surface.shadowSm,
                fontWeight: 600,
              },
            },
          },
          MuiCheckbox: {
            styleOverrides: {
              root: {
                borderRadius: 12,
              },
            },
          },
          MuiIconButton: {
            styleOverrides: {
              root: {
                borderRadius: 16,
                border: `1px solid ${surface.border}`,
                backgroundImage: surface.panelSoft,
                boxShadow: surface.shadowSm,
              },
            },
          },
          MuiOutlinedInput: {
            styleOverrides: {
              root: {
                borderRadius: 22,
                border: `1px solid ${surface.border}`,
                backgroundImage: surface.panelSoft,
                boxShadow: surface.shadowInset,
                '& fieldset': {
                  border: 'none',
                },
              },
              input: {
                paddingBlock: 13,
              },
            },
          },
          MuiSelect: {
            styleOverrides: {
              icon: {
                right: 14,
              },
              select: {
                borderRadius: 22,
              },
            },
          },
          MuiInputLabel: {
            styleOverrides: {
              root: {
                color: textSecondary,
              },
            },
          },
          MuiInputBase: {
            styleOverrides: {
              input: {
                '&::placeholder': {
                  color: textSecondary,
                  opacity: 1,
                },
              },
            },
          },
          MuiListItemButton: {
            styleOverrides: {
              root: {
                borderRadius: 22,
              },
            },
          },
          MuiMenuItem: {
            styleOverrides: {
              root: {
                borderRadius: 16,
                marginInline: 6,
                marginBlock: 4,
                minHeight: 38,
              },
            },
          },
          MuiMenu: {
            styleOverrides: {
              paper: {
                backgroundImage: surface.panelSoft,
                boxShadow: surface.shadow,
                borderRadius: 24,
              },
            },
          },
          MuiSkeleton: {
            styleOverrides: {
              root: {
                borderRadius: 20,
              },
            },
          },
          MuiTabs: {
            styleOverrides: {
              root: {
                minHeight: 52,
              },
              indicator: {
                height: 0,
              },
            },
          },
          MuiTab: {
            styleOverrides: {
              root: {
                minHeight: 44,
                borderRadius: 18,
                margin: 4,
                transition: 'all 180ms ease',
                '&.Mui-selected': {
                  backgroundImage: `linear-gradient(135deg, ${alpha(primaryMain, 0.18)}, rgba(255,255,255,0.02))`,
                  boxShadow: surface.shadowInset,
                },
              },
            },
          },
          MuiToggleButtonGroup: {
            styleOverrides: {
              root: {
                gap: 8,
                padding: 6,
                borderRadius: 22,
                backgroundImage: surface.panelInset,
                boxShadow: surface.shadowInset,
              },
              grouped: {
                borderRadius: 16,
                border: 'none',
                margin: 0,
              },
            },
          },
          MuiToggleButton: {
            styleOverrides: {
              root: {
                borderRadius: 16,
                border: `1px solid ${surface.border}`,
                boxShadow: surface.shadowSm,
                '&.Mui-selected': {
                  backgroundImage: `linear-gradient(135deg, ${alpha(primaryMain, 0.22)}, rgba(255,255,255,0.04))`,
                  boxShadow: surface.shadowInset,
                },
              },
            },
          },
          MuiLinearProgress: {
            styleOverrides: {
              root: {
                height: 10,
                borderRadius: 999,
                backgroundColor: alpha(
                  primaryMain,
                  mode === 'light' ? 0.12 : 0.18,
                ),
              },
              bar: {
                borderRadius: 999,
              },
            },
          },
          MuiTableContainer: {
            styleOverrides: {
              root: {
                borderRadius: 26,
                border: `1px solid ${surface.border}`,
                backgroundImage: surface.panelSoft,
                boxShadow: surface.shadow,
              },
            },
          },
          MuiTableCell: {
            styleOverrides: {
              root: {
                borderColor: alpha(
                  mode === 'light' ? '#24324A' : '#FFFFFF',
                  0.08,
                ),
              },
              head: {
                fontWeight: 700,
                color: textSecondary,
              },
            },
          },
          MuiTableRow: {
            styleOverrides: {
              root: {
                '&:hover': {
                  backgroundColor: alpha(
                    primaryMain,
                    mode === 'light' ? 0.06 : 0.08,
                  ),
                },
              },
            },
          },
          MuiFormControlLabel: {
            styleOverrides: {
              label: {
                fontWeight: 500,
              },
            },
          },
          MuiAlert: {
            styleOverrides: {
              root: {
                borderRadius: 22,
                border: `1px solid ${surface.border}`,
                boxShadow: surface.shadowSm,
              },
            },
          },
          MuiTooltip: {
            styleOverrides: {
              tooltip: {
                borderRadius: 14,
                backgroundColor: alpha(
                  mode === 'light' ? '#20304D' : '#111827',
                  0.9,
                ),
                padding: '8px 10px',
              },
            },
          },
        },
      })
    } catch (e) {
      console.error('Error creating MUI theme, falling back to defaults:', e)
      muiTheme = createTheme({
        breakpoints: {
          values: { xs: 0, sm: 650, md: 900, lg: 1200, xl: 1536 },
        },
        palette: {
          mode,
          primary: { main: dt.primary_color },
          secondary: { main: dt.secondary_color },
          info: { main: dt.info_color },
          error: { main: dt.error_color },
          warning: { main: dt.warning_color },
          success: { main: dt.success_color },
          text: { primary: dt.primary_text, secondary: dt.secondary_text },
          background: {
            paper: surface.panel,
            default: surface.canvas,
          },
        },
        shape: {
          borderRadius: 26,
        },
        shadows: buildShadows(surface),
        typography: { fontFamily: dt.font_family },
      })
    }

    const rootEle = document.documentElement
    if (rootEle) {
      const backgroundColor = surface.canvas
      const selectColor = mode === 'light' ? '#f5f5f5' : '#3E3E3E'
      const scrollColor = mode === 'light' ? '#90939980' : '#555555'
      const dividerColor =
        mode === 'light' ? 'rgba(0, 0, 0, 0.06)' : 'rgba(255, 255, 255, 0.06)'
      rootEle.style.setProperty(
        '--app-font-family',
        String(muiTheme.typography.fontFamily ?? dt.font_family),
      )
      rootEle.style.setProperty('--divider-color', dividerColor)
      rootEle.style.setProperty('--background-color', backgroundColor)
      rootEle.style.setProperty('--surface-panel', surface.panelSoft)
      rootEle.style.setProperty('--surface-panel-inset', surface.panelInset)
      rootEle.style.setProperty('--surface-border', surface.border)
      rootEle.style.setProperty('--surface-outline', surface.outline)
      rootEle.style.setProperty('--shadow-raised', surface.shadow)
      rootEle.style.setProperty('--shadow-raised-sm', surface.shadowSm)
      rootEle.style.setProperty('--shadow-inset', surface.shadowInset)
      rootEle.style.setProperty('--app-shell-background', surface.shell)
      rootEle.style.setProperty('--selection-color', selectColor)
      rootEle.style.setProperty('--scroller-color', scrollColor)
      rootEle.style.setProperty('--primary-main', muiTheme.palette.primary.main)
      rootEle.style.setProperty(
        '--background-color-alpha',
        alpha(muiTheme.palette.primary.main, 0.1),
      )
      rootEle.style.setProperty(
        '--window-border-color',
        mode === 'light' ? '#cccccc' : '#1E1E1E',
      )
      rootEle.style.setProperty(
        '--scrollbar-bg',
        mode === 'light' ? '#f1f1f1' : '#2E303D',
      )
      rootEle.style.setProperty(
        '--scrollbar-thumb',
        mode === 'light' ? '#c1c1c1' : '#555555',
      )
      rootEle.style.setProperty(
        '--user-background-image',
        hasUserBackground ? `url('${userBackgroundImage}')` : 'none',
      )
      rootEle.style.setProperty(
        '--background-blend-mode',
        setting.background_blend_mode || 'normal',
      )
      rootEle.style.setProperty(
        '--background-opacity',
        setting.background_opacity !== undefined
          ? String(setting.background_opacity)
          : '1',
      )
      rootEle.setAttribute('data-css-injection-root', 'true')
    }

    let styleElement = document.querySelector('style#verge-theme')
    if (!styleElement) {
      styleElement = document.createElement('style')
      styleElement.id = 'verge-theme'
      document.head.appendChild(styleElement)
    }

    if (styleElement) {
      let scopedCss: string | null = null
      if (canUseCssScope() && setting.css_injection) {
        scopedCss = wrapCssInjectionWithScope(setting.css_injection)
      }
      const effectiveInjectedCss = scopedCss ?? setting.css_injection ?? ''
      const globalStyles = `
        ::-webkit-scrollbar {
          width: 8px;
          height: 8px;
          background-color: transparent;
        }
        ::-webkit-scrollbar-thumb {
          background-color: var(--scrollbar-thumb);
          border-radius: 999px;
          border: 2px solid transparent;
          background-clip: padding-box;
        }
        ::-webkit-scrollbar-thumb:hover {
          background-color: ${mode === 'light' ? '#a1a1a1' : '#666666'};
        }

        body {
          font-family: var(--app-font-family);
          color: ${muiTheme.palette.text.primary};
          background: var(--app-shell-background);
          ${
            hasUserBackground
              ? `
            background-image: var(--user-background-image), var(--app-shell-background);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            background-blend-mode: var(--background-blend-mode);
          `
              : ''
          }
        }

        body::before {
          content: '';
          position: fixed;
          inset: 0;
          pointer-events: none;
          background:
            radial-gradient(circle at 18% 16%, ${alpha(primaryMain, mode === 'light' ? 0.12 : 0.16)} 0, transparent 26%),
            radial-gradient(circle at 85% 12%, ${alpha(secondaryMain, mode === 'light' ? 0.12 : 0.1)} 0, transparent 28%);
          z-index: 0;
        }

        #root {
          position: relative;
          z-index: 1;
        }

        *:focus-visible {
          outline: 2px solid ${alpha(primaryMain, 0.72)};
          outline-offset: 2px;
        }
      `

      styleElement.innerHTML = effectiveInjectedCss + globalStyles
    }

    return muiTheme
  }, [mode, theme_setting, userBackgroundImage, hasUserBackground])

  useEffect(() => {
    const id = setTimeout(() => {
      const dom = document.querySelector('#Gradient2')
      if (dom) {
        dom.innerHTML = `
        <stop offset="0%" stop-color="${theme.palette.primary.main}" />
        <stop offset="80%" stop-color="${theme.palette.primary.dark}" />
        <stop offset="100%" stop-color="${theme.palette.primary.dark}" />
        `
      }
    }, 0)
    return () => clearTimeout(id)
  }, [theme.palette.primary.main, theme.palette.primary.dark])

  return { theme }
}
