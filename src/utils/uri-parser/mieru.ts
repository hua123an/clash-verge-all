import {
  decodeAndTrim,
  getIfNotBlank,
  parseBoolOrPresence,
  parsePortOrDefault,
  parseQueryStringNormalized,
  parseUrlLike,
  safeDecodeURIComponent,
  splitOnce,
  stripUriScheme,
} from './helpers'

const MIERU_TRANSPORTS = ['TCP', 'UDP'] as const
const MIERU_MULTIPLEXING = [
  'MULTIPLEXING_OFF',
  'MULTIPLEXING_LOW',
  'MULTIPLEXING_MIDDLE',
  'MULTIPLEXING_HIGH',
] as const

function parseMieruTransport(
  value: string | undefined,
): MieruTransport | undefined {
  const normalized = value?.trim().toUpperCase()
  return normalized && MIERU_TRANSPORTS.includes(normalized as MieruTransport)
    ? (normalized as MieruTransport)
    : undefined
}

function parseMieruMultiplexing(
  value: string | undefined,
): MieruMultiplexing | undefined {
  const normalized = value?.trim().toUpperCase()
  return normalized &&
    MIERU_MULTIPLEXING.includes(normalized as MieruMultiplexing)
    ? (normalized as MieruMultiplexing)
    : undefined
}

export function URI_Mieru(line: string): IProxyMieruConfig {
  const afterScheme = stripUriScheme(line, 'mieru', 'Invalid mieru uri')
  if (!afterScheme) {
    throw new Error('Invalid mieru uri')
  }

  const {
    auth: authRaw,
    host: server,
    port,
    query: addons,
    fragment: nameRaw,
  } = parseUrlLike(afterScheme, { errorMessage: 'Invalid mieru uri' })
  if (!server) {
    throw new Error('Invalid mieru uri')
  }

  const portNum = parsePortOrDefault(port, 443)
  const auth = safeDecodeURIComponent(authRaw) ?? authRaw
  const name = decodeAndTrim(nameRaw) ?? `Mieru ${server}:${portNum}`

  const proxy: IProxyMieruConfig = {
    type: 'mieru',
    name,
    server,
    port: portNum,
  }

  if (auth) {
    const [username, password] = splitOnce(auth, ':')
    const parsedUsername = getIfNotBlank(username)
    const parsedPassword = getIfNotBlank(password)
    if (parsedUsername) {
      proxy.username = parsedUsername
    }
    if (parsedPassword) {
      proxy.password = parsedPassword
    }
  }

  const params = parseQueryStringNormalized(addons)
  const transport = parseMieruTransport(params.transport)
  if (transport) {
    proxy.transport = transport
  }

  const multiplexing = parseMieruMultiplexing(params.multiplexing)
  if (multiplexing) {
    proxy.multiplexing = multiplexing
  }

  const username = getIfNotBlank(params.username)
  if (username) {
    proxy.username = username
  }

  const password = getIfNotBlank(params.password)
  if (password) {
    proxy.password = password
  }

  const portRange = getIfNotBlank(params['port-range'])
  if (portRange) {
    proxy['port-range'] = portRange
  }

  const handshakeMode = getIfNotBlank(params['handshake-mode'])
  if (handshakeMode) {
    proxy['handshake-mode'] = handshakeMode
  }

  if (Object.hasOwn(params, 'udp')) {
    proxy.udp = parseBoolOrPresence(params.udp)
  }

  return proxy
}
