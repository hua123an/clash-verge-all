import {
  decodeAndTrim,
  parsePortOrDefault,
  parseQueryStringNormalized,
  parseUrlLike,
  safeDecodeURIComponent,
  splitOnce,
  stripUriScheme,
} from './helpers'

export function URI_SSH(line: string): IProxySshConfig {
  const afterScheme = stripUriScheme(line, 'ssh', 'Invalid ssh uri')
  if (!afterScheme) {
    throw new Error('Invalid ssh uri')
  }

  const {
    auth: authRaw,
    host: server,
    port,
    query: addons,
    fragment: nameRaw,
  } = parseUrlLike(afterScheme, {
    errorMessage: 'Invalid ssh uri',
  })

  const portNum = parsePortOrDefault(port, 22)
  const auth = safeDecodeURIComponent(authRaw) ?? authRaw
  const name = decodeAndTrim(nameRaw) ?? `SSH ${server}:${portNum}`

  const proxy: IProxySshConfig = {
    type: 'ssh',
    name,
    server,
    port: portNum,
  }

  if (auth) {
    const [username, password] = splitOnce(auth, ':')
    if (username) proxy.username = username
    if (password) proxy.password = password
  }

  const params = parseQueryStringNormalized(addons)
  if (params['private-key']) {
    proxy['private-key'] = params['private-key']
  }
  if (params['private-key-passphrase']) {
    proxy['private-key-passphrase'] = params['private-key-passphrase']
  }
  if (params['host-key']) {
    proxy['host-key'] = params['host-key']
  }
  if (params['host-key-algorithms']) {
    proxy['host-key-algorithms'] = params['host-key-algorithms']
  }

  return proxy
}
