import {
  ArrowDownwardRounded,
  ArrowUpwardRounded,
  CloudDownloadRounded,
  CloudUploadRounded,
  LinkRounded,
  MemoryRounded,
} from '@mui/icons-material'
import {
  Grid,
  PaletteColor,
  Paper,
  Typography,
  alpha,
  useTheme,
} from '@mui/material'
import { ReactNode, memo, useMemo, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import { TrafficErrorBoundary } from '@/components/shared/traffic-error-boundary'
import { useConnectionData } from '@/hooks/use-connection-data'
import { useMemoryData } from '@/hooks/use-memory-data'
import { useTrafficData } from '@/hooks/use-traffic-data'
import { useVerge } from '@/hooks/use-verge'
import { useVisibility } from '@/hooks/use-visibility'
import parseTraffic from '@/utils/parse-traffic'

import {
  EnhancedCanvasTrafficGraph,
  type EnhancedCanvasTrafficGraphRef,
} from './enhanced-canvas-traffic-graph'

interface StatCardProps {
  icon: ReactNode
  title: string
  value: string | number
  unit: string
  color: 'primary' | 'secondary' | 'error' | 'warning' | 'info' | 'success'
  onClick?: () => void
}

// 全局变量类型定义
declare global {
  interface Window {
    animationFrameId?: number
    lastTrafficData?: {
      up: number
      down: number
    }
  }
}

// 统计卡片组件 - 使用memo优化
const CompactStatCard = memo(
  ({ icon, title, value, unit, color, onClick }: StatCardProps) => {
    const theme = useTheme()

    // 获取调色板颜色 - 使用useMemo避免重复计算
    const colorValue = useMemo(() => {
      const palette = theme.palette
      if (
        color in palette &&
        palette[color as keyof typeof palette] &&
        'main' in (palette[color as keyof typeof palette] as PaletteColor)
      ) {
        return (palette[color as keyof typeof palette] as PaletteColor).main
      }
      return palette.primary.main
    }, [theme.palette, color])

    return (
      <Paper
        elevation={0}
        sx={{
          display: 'flex',
          alignItems: 'center',
          borderRadius: '22px',
          bgcolor: 'transparent',
          background: `linear-gradient(145deg, ${alpha(colorValue, 0.1)}, rgba(255,255,255,0.02))`,
          border: '1px solid var(--surface-border)',
          padding: '8px',
          transition: 'all 0.2s ease-in-out',
          boxShadow: 'var(--shadow-raised-sm)',
          cursor: onClick ? 'pointer' : 'default',
          '&:hover': onClick
            ? {
                boxShadow: 'var(--shadow-inset)',
              }
            : {},
        }}
        onClick={onClick}
      >
        {/* 图标容器 */}
        <Grid
          component="div"
          sx={{
            mr: 1,
            ml: '2px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            width: 32,
            height: 32,
            borderRadius: '14px',
            bgcolor: alpha(colorValue, 0.1),
            color: colorValue,
            boxShadow: 'var(--shadow-inset)',
          }}
        >
          {icon}
        </Grid>

        {/* 文本内容 */}
        <Grid component="div" sx={{ flexGrow: 1, minWidth: 0 }}>
          <Typography variant="caption" color="text.secondary" noWrap>
            {title}
          </Typography>
          <Grid
            component="div"
            sx={{ display: 'flex', alignItems: 'baseline' }}
          >
            <Typography
              variant="body1"
              noWrap
              sx={{ mr: 0.5, fontWeight: 'bold' }}
            >
              {value}
            </Typography>
            <Typography variant="caption" color="text.secondary">
              {unit}
            </Typography>
          </Grid>
        </Grid>
      </Paper>
    )
  },
)

// 添加显示名称
CompactStatCard.displayName = 'CompactStatCard'

export const EnhancedTrafficStats = () => {
  const { t } = useTranslation()
  const { verge } = useVerge()
  const trafficRef = useRef<EnhancedCanvasTrafficGraphRef>(null)
  const pageVisible = useVisibility()

  // 是否显示流量图表
  const trafficGraph = verge?.traffic_graph ?? true

  const {
    response: { data: traffic },
  } = useTrafficData({ enabled: trafficGraph && pageVisible })

  const {
    response: { data: memory },
  } = useMemoryData()

  const {
    response: { data: connections },
  } = useConnectionData()

  // Canvas组件现在直接从全局Hook获取数据，无需手动添加数据点

  // 使用useMemo计算解析后的流量数据
  const parsedData = useMemo(() => {
    const [up, upUnit] = parseTraffic(traffic?.up || 0)
    const [down, downUnit] = parseTraffic(traffic?.down || 0)
    const [inuse, inuseUnit] = parseTraffic(memory?.inuse || 0)
    const [uploadTotal, uploadTotalUnit] = parseTraffic(
      connections?.uploadTotal,
    )
    const [downloadTotal, downloadTotalUnit] = parseTraffic(
      connections?.downloadTotal,
    )

    return {
      up,
      upUnit,
      down,
      downUnit,
      inuse,
      inuseUnit,
      uploadTotal,
      uploadTotalUnit,
      downloadTotal,
      downloadTotalUnit,
      connectionsCount: connections?.activeConnections.length,
    }
  }, [traffic, memory, connections])

  // 渲染流量图表 - 使用useMemo缓存渲染结果
  const trafficGraphComponent = useMemo(() => {
    if (!trafficGraph || !pageVisible) return null

    return (
      <Paper
        elevation={0}
        sx={{
          height: 130,
          cursor: 'pointer',
          border: '1px solid var(--surface-border)',
          borderRadius: '24px',
          overflow: 'hidden',
          background: 'var(--surface-panel-inset)',
          boxShadow: 'var(--shadow-inset)',
        }}
        onClick={() => trafficRef.current?.toggleStyle()}
      >
        <div style={{ height: '100%', position: 'relative' }}>
          <EnhancedCanvasTrafficGraph ref={trafficRef} />
        </div>
      </Paper>
    )
  }, [trafficGraph, pageVisible])

  // 使用useMemo计算统计卡片配置
  const statCards = useMemo(
    () => [
      {
        icon: <ArrowUpwardRounded fontSize="small" />,
        title: t('home.components.traffic.metrics.uploadSpeed'),
        value: parsedData.up,
        unit: `${parsedData.upUnit}/s`,
        color: 'secondary' as const,
      },
      {
        icon: <ArrowDownwardRounded fontSize="small" />,
        title: t('home.components.traffic.metrics.downloadSpeed'),
        value: parsedData.down,
        unit: `${parsedData.downUnit}/s`,
        color: 'primary' as const,
      },
      {
        icon: <LinkRounded fontSize="small" />,
        title: t('home.components.traffic.metrics.activeConnections'),
        value: parsedData.connectionsCount,
        unit: '',
        color: 'success' as const,
      },
      {
        icon: <CloudUploadRounded fontSize="small" />,
        title: t('shared.labels.uploaded'),
        value: parsedData.uploadTotal,
        unit: parsedData.uploadTotalUnit,
        color: 'secondary' as const,
      },
      {
        icon: <CloudDownloadRounded fontSize="small" />,
        title: t('shared.labels.downloaded'),
        value: parsedData.downloadTotal,
        unit: parsedData.downloadTotalUnit,
        color: 'primary' as const,
      },
      {
        icon: <MemoryRounded fontSize="small" />,
        title: t('home.components.traffic.metrics.memoryUsage'),
        value: parsedData.inuse,
        unit: parsedData.inuseUnit,
        color: 'error' as const,
        onClick: undefined,
      },
    ],
    [t, parsedData],
  )

  return (
    <TrafficErrorBoundary
      onError={(error, errorInfo) => {
        console.error('[EnhancedTrafficStats] 组件错误:', error, errorInfo)
      }}
    >
      <Grid container spacing={1} columns={{ xs: 8, sm: 8, md: 12 }}>
        {trafficGraph && (
          <Grid size={12}>
            {/* 流量图表区域 */}
            {trafficGraphComponent}
          </Grid>
        )}
        {/* 统计卡片区域 */}
        {statCards.map((card) => (
          <Grid key={card.title} size={4}>
            <CompactStatCard {...(card as StatCardProps)} />
          </Grid>
        ))}
      </Grid>
    </TrafficErrorBoundary>
  )
}
