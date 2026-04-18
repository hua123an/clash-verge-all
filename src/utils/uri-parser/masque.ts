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

export function URI_Masque(line: string): IProxyMasqueConfig {
  const afterScheme = stripUriScheme(line, 'masque', 'Invalid masque uri')
  if (!afterScheme) {
    throw new Error('Invalid masque uri')
  }

  const {
    auth: privateKeyRaw,
    host: server,
    port,
    query: addons,
    fragment: nameRaw,
  } = parseUrlLike(afterScheme, { errorMessage: 'Invalid masque uri' })
  if (!server) {
    throw new Error('Invalid masque uri')
  }

  const portNum = parsePortOrDefault(port, 443)
  const privateKey = safeDecodeURIComponent(privateKeyRaw) ?? privateKeyRaw
  const name = decodeAndTrim(nameRaw) ?? `Masque ${server}:${portNum}`

  const proxy: IProxyMasqueConfig = {
    type: 'masque',
    name,
    server,
    port: portNum,
  }

  const parsedPrivateKey = getIfNotBlank(privateKey)
  if (parsedPrivateKey) {
    proxy['private-key'] = parsedPrivateKey
  }

  const params = parseQueryStringNormalized(addons)

  const queryPrivateKey = getIfNotBlank(params['private-key'])
  if (queryPrivateKey) {
    proxy['private-key'] = queryPrivateKey
  }

  const publicKey = getIfNotBlank(params['public-key'])
  if (publicKey) {
    proxy['public-key'] = publicKey
  }

  const ip = getIfNotBlank(params.ip)
  if (ip) {
    proxy.ip = ip
  }

  const ipv6 = getIfNotBlank(params.ipv6)
  if (ipv6) {
    proxy.ipv6 = ipv6
  }

  const mtu = parseInteger(params.mtu)
  if (mtu !== undefined) {
    proxy.mtu = mtu
  }

  if (Object.hasOwn(params, 'udp')) {
    proxy.udp = parseBoolOrPresence(params.udp)
  }

  if (Object.hasOwn(params, 'remote-dns-resolve')) {
    proxy['remote-dns-resolve'] = parseBoolOrPresence(
      params['remote-dns-resolve'],
    )
  }

  const dns = params.dns
    ?.split(',')
    .map((item) => item.trim())
    .filter(Boolean)
  if (dns?.length) {
    proxy.dns = dns
  }

  return proxy
}
