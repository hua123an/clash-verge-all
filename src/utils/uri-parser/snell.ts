import {
  decodeAndTrim,
  parseBoolOrPresence,
  parseInteger,
  parsePortOrDefault,
  parseQueryStringNormalized,
  parseUrlLike,
  safeDecodeURIComponent,
  stripUriScheme,
} from './helpers'

export function URI_Snell(line: string): IProxySnellConfig {
  const afterScheme = stripUriScheme(line, 'snell', 'Invalid snell uri')
  if (!afterScheme) {
    throw new Error('Invalid snell uri')
  }

  const {
    auth: authRaw,
    host: server,
    port,
    query: addons,
    fragment: nameRaw,
  } = parseUrlLike(afterScheme, {
    errorMessage: 'Invalid snell uri',
  })

  const portNum = parsePortOrDefault(port, 443)
  const auth = safeDecodeURIComponent(authRaw) ?? authRaw
  const name = decodeAndTrim(nameRaw) ?? `Snell ${server}:${portNum}`

  const proxy: IProxySnellConfig = {
    type: 'snell',
    name,
    server,
    port: portNum,
  }

  if (auth) {
    proxy.psk = auth
  }

  const params = parseQueryStringNormalized(addons)
  const version = parseInteger(params.version)
  if (version !== undefined) {
    proxy.version = version
  }
  if (Object.prototype.hasOwnProperty.call(params, 'udp')) {
    proxy.udp = parseBoolOrPresence(params.udp)
  }

  return proxy
}
