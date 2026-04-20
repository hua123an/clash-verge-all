import getSystem from '@/utils/get-system'
const OS = getSystem()

// default theme setting
export const defaultTheme = {
  primary_color: '#4A78F6',
  secondary_color: '#FF9D6C',
  primary_text: '#162033',
  secondary_text: '#55607A',
  info_color: '#4A78F6',
  error_color: '#E45C57',
  warning_color: '#F2A649',
  success_color: '#34A07F',
  background_color: '#E7EDF3',
  font_family: `"SF Pro Display", "Avenir Next", "Segoe UI Variable Display", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei UI", "Noto Sans SC", sans-serif${
    OS === 'windows' ? ', twemoji mozilla' : ''
  }`,
}

// dark mode
export const defaultDarkTheme = {
  ...defaultTheme,
  primary_color: '#7A97FF',
  secondary_color: '#FFB077',
  primary_text: '#F2F5FF',
  background_color: '#1E2430',
  secondary_text: '#9EA8C4',
  info_color: '#7A97FF',
  error_color: '#FF7670',
  warning_color: '#F3B35F',
  success_color: '#49C7A0',
}
