import {
  ArrowDownwardRounded,
  ArrowUpwardRounded,
  MemoryRounded,
} from '@mui/icons-material'
import { Box, Typography } from '@mui/material'
import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'

import { LightweightTrafficErrorBoundary } from '@/components/shared/traffic-error-boundary'
import { useMemoryData } from '@/hooks/use-memory-data'
import { useTrafficData } from '@/hooks/use-traffic-data'
import { useVerge } from '@/hooks/use-verge'
import { useVisibility } from '@/hooks/use-visibility'
import parseTraffic from '@/utils/parse-traffic'

import { TrafficGraph, type TrafficRef } from './traffic-graph'

// setup the traffic
export const LayoutTraffic = () => {
  const { t } = useTranslation()
  const { verge } = useVerge()

  // whether hide traffic graph
  const trafficGraph = verge?.traffic_graph ?? true

  const trafficRef = useRef<TrafficRef>(null)
  const pageVisible = useVisibility()

  const {
    response: { data: traffic },
  } = useTrafficData({ enabled: trafficGraph && pageVisible })
  const {
    response: { data: memory },
  } = useMemoryData()

  // 监听数据变化，为图表添加数据点
  useEffect(() => {
    if (trafficRef.current) {
      trafficRef.current.appendData({
        up: traffic?.up || 0,
        down: traffic?.down || 0,
      })
    }
  }, [traffic])

  // 显示内存使用情况的设置
  const displayMemory = verge?.enable_memory_usage ?? true

  // 使用parseTraffic统一处理转换，保持与首页一致的显示格式
  const [up, upUnit] = parseTraffic(traffic?.up || 0)
  const [down, downUnit] = parseTraffic(traffic?.down || 0)
  const [inuse, inuseUnit] = parseTraffic(memory?.inuse || 0)

  const rowSx = {
    display: 'flex',
    alignItems: 'center',
    whiteSpace: 'nowrap',
  }
  const iconSx = { mr: '8px', fontSize: 16 }
  const valueTypographyProps = {
    component: 'span' as const,
    sx: { flex: '1 1 56px', userSelect: 'none', textAlign: 'center' },
  }
  const unitTypographyProps = {
    component: 'span' as const,
    color: 'grey.500',
    sx: {
      flex: '0 1 27px',
      userSelect: 'none',
      fontSize: '12px',
      textAlign: 'right',
    },
  }

  return (
    <LightweightTrafficErrorBoundary>
      <Box
        sx={{
          position: 'relative',
          borderRadius: '22px',
        }}
      >
        {trafficGraph && pageVisible && (
          <Box
            role="button"
            tabIndex={0}
            style={{ width: '100%', height: 64, marginBottom: 10 }}
            onClick={trafficRef.current?.toggleStyle}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault()
                trafficRef.current?.toggleStyle()
              }
            }}
          >
            <TrafficGraph ref={trafficRef} />
          </Box>
        )}

        <Box sx={{ display: 'flex', flexDirection: 'column', gap: 1 }}>
          <Box
            title={`${t('home.components.traffic.metrics.uploadSpeed')}`}
            sx={{
              ...rowSx,
            }}
          >
            <ArrowUpwardRounded
              sx={iconSx}
              color={(traffic?.up || 0) > 0 ? 'secondary' : 'disabled'}
            />
            <Typography {...valueTypographyProps} color="secondary">
              {up}
            </Typography>
            <Typography {...unitTypographyProps}>{upUnit}/s</Typography>
          </Box>

          <Box
            title={`${t('home.components.traffic.metrics.downloadSpeed')}`}
            sx={{
              ...rowSx,
            }}
          >
            <ArrowDownwardRounded
              sx={iconSx}
              color={(traffic?.down || 0) > 0 ? 'primary' : 'disabled'}
            />
            <Typography {...valueTypographyProps} color="primary">
              {down}
            </Typography>
            <Typography {...unitTypographyProps}>{downUnit}/s</Typography>
          </Box>

          {displayMemory && (
            <Box
              title={`${t('home.components.traffic.metrics.memoryUsage')} `}
              sx={{
                ...rowSx,
                cursor: 'auto',
              }}
              color={'disabled'}
              onClick={async () => {
                // isDebug && (await gc());
              }}
            >
              <MemoryRounded sx={iconSx} />
              <Typography {...valueTypographyProps}>{inuse}</Typography>
              <Typography {...unitTypographyProps}>{inuseUnit}</Typography>
            </Box>
          )}
        </Box>
      </Box>
    </LightweightTrafficErrorBoundary>
  )
}
