import {
  decodeAndTrim,
  getIfNotBlank,
  parseBoolOrPresence,
  parseInteger,
  parsePortOrDefault,
  parseQueryStringNormalized,
  parseUrlLike,
  safeDecodeURIComponent,
  stripUriScheme,
} from './helpers'

const SUDOKU_AEAD_METHODS = [
  'chacha20-poly1305',
  'aes-128-gcm',
  'none',
] as const
const SUDOKU_TABLE_TYPES = ['prefer_ascii', 'prefer_entropy'] as const
const SUDOKU_HTTP_MASK_MODES = ['legacy', 'stream', 'poll', 'auto'] as const
const SUDOKU_HTTP_MASK_STRATEGIES = ['random', 'post', 'websocket'] as const

function parseSudokuAeadMethod(
  value: string | undefined,
): SudokuAeadMethod | undefined {
  const normalized = value?.trim().toLowerCase()
  return normalized &&
    SUDOKU_AEAD_METHODS.includes(normalized as SudokuAeadMethod)
    ? (normalized as SudokuAeadMethod)
    : undefined
}

function parseSudokuTableType(
  value: string | undefined,
): SudokuTableType | undefined {
  const normalized = value?.trim().toLowerCase()
  return normalized &&
    SUDOKU_TABLE_TYPES.includes(normalized as SudokuTableType)
    ? (normalized as SudokuTableType)
    : undefined
}

function parseSudokuHttpMaskMode(
  value: string | undefined,
): SudokuHttpMaskMode | undefined {
  const normalized = value?.trim().toLowerCase()
  return normalized &&
    SUDOKU_HTTP_MASK_MODES.includes(normalized as SudokuHttpMaskMode)
    ? (normalized as SudokuHttpMaskMode)
    : undefined
}

function parseSudokuHttpMaskStrategy(
  value: string | undefined,
): SudokuHttpMaskStrategy | undefined {
  const normalized = value?.trim().toLowerCase()
  return normalized &&
    SUDOKU_HTTP_MASK_STRATEGIES.includes(normalized as SudokuHttpMaskStrategy)
    ? (normalized as SudokuHttpMaskStrategy)
    : undefined
}

export function URI_Sudoku(line: string): IProxySudokuConfig {
  const afterScheme = stripUriScheme(line, 'sudoku', 'Invalid sudoku uri')
  if (!afterScheme) {
    throw new Error('Invalid sudoku uri')
  }

  const {
    auth: keyRaw,
    host: server,
    port,
    query: addons,
    fragment: nameRaw,
  } = parseUrlLike(afterScheme, { errorMessage: 'Invalid sudoku uri' })
  if (!server) {
    throw new Error('Invalid sudoku uri')
  }

  const portNum = parsePortOrDefault(port, 443)
  const key = safeDecodeURIComponent(keyRaw) ?? keyRaw
  const name = decodeAndTrim(nameRaw) ?? `Sudoku ${server}:${portNum}`

  const proxy: IProxySudokuConfig = {
    type: 'sudoku',
    name,
    server,
    port: portNum,
  }

  const parsedKey = getIfNotBlank(key)
  if (parsedKey) {
    proxy.key = parsedKey
  }

  const params = parseQueryStringNormalized(addons)

  const queryKey = getIfNotBlank(params.key)
  if (queryKey) {
    proxy.key = queryKey
  }

  const aeadMethod = parseSudokuAeadMethod(params['aead-method'])
  if (aeadMethod) {
    proxy['aead-method'] = aeadMethod
  }

  const paddingMin = parseInteger(params['padding-min'])
  if (paddingMin !== undefined) {
    proxy['padding-min'] = paddingMin
  }

  const paddingMax = parseInteger(params['padding-max'])
  if (paddingMax !== undefined) {
    proxy['padding-max'] = paddingMax
  }

  const tableType = parseSudokuTableType(params['table-type'])
  if (tableType) {
    proxy['table-type'] = tableType
  }

  if (Object.hasOwn(params, 'enable-pure-downlink')) {
    proxy['enable-pure-downlink'] = parseBoolOrPresence(
      params['enable-pure-downlink'],
    )
  }

  if (Object.hasOwn(params, 'http-mask')) {
    proxy['http-mask'] = parseBoolOrPresence(params['http-mask'])
  }

  const httpMaskMode = parseSudokuHttpMaskMode(params['http-mask-mode'])
  if (httpMaskMode) {
    proxy['http-mask-mode'] = httpMaskMode
  }

  if (Object.hasOwn(params, 'http-mask-tls')) {
    proxy['http-mask-tls'] = parseBoolOrPresence(params['http-mask-tls'])
  }

  const httpMaskHost = getIfNotBlank(params['http-mask-host'])
  if (httpMaskHost) {
    proxy['http-mask-host'] = httpMaskHost
  }

  const httpMaskStrategy = parseSudokuHttpMaskStrategy(
    params['http-mask-strategy'],
  )
  if (httpMaskStrategy) {
    proxy['http-mask-strategy'] = httpMaskStrategy
  }

  const customTable = getIfNotBlank(params['custom-table'])
  if (customTable) {
    proxy['custom-table'] = customTable
  }

  const customTables = params['custom-tables']
    ?.split(',')
    .map((item) => item.trim())
    .filter(Boolean)
  if (customTables?.length) {
    proxy['custom-tables'] = customTables
  }

  return proxy
}
