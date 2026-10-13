import { afterAll, afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test'
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve, sep } from 'node:path'

import { zipSync, strToU8 } from 'fflate'

const upload = mock(async () => ({ id: 42 }))
let elapsed = 0
mock.module('@actions/artifact', () => ({
  DefaultArtifactClient: class {
    uploadArtifact = upload
  },
}))
mock.module('node:timers/promises', () => ({
  setTimeout: async (delay: number) => {
    elapsed += delay
  },
}))
const { signFile } = await import('./signpath')

const environment = {
  GITHUB_ACTIONS: 'true',
  GITHUB_RUN_ID: '123',
  GITHUB_REPOSITORY: 'koharu-rs/koharu',
  GITHUB_TOKEN: 'github-token',
  SIGNPATH_ORGANIZATION_ID: 'test-organization',
  SIGNPATH_API_TOKEN: 'submit-token',
  SIGNPATH_APPROVER_API_TOKEN: 'approve-token',
  SIGNPATH_SIGNING_POLICY_SLUG: 'api-approval-policy',
  SIGNPATH_ARTIFACT_CONFIGURATION_SLUG: 'inline-file',
}
const originalEnvironment = { ...process.env }
const fetchMock = spyOn(globalThis, 'fetch')
const clock = spyOn(Date, 'now')
const logger = spyOn(console, 'log').mockImplementation(() => {})
const calls: { url: string; init: RequestInit }[] = []
let directory: string
let file: string
let submitted: object
let statuses: { status: string; isFinalStatus: boolean }[]
let approvalStatus: number
let archive: Uint8Array

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), 'koharu-signpath-test-'))
  file = join(directory, 'app with spaces.exe')
  await writeFile(file, 'unsigned bytes')
  Object.assign(process.env, environment)
  elapsed = 0
  clock.mockImplementation(() => Date.UTC(2026, 9, 14) + elapsed)
  calls.length = 0
  upload.mockClear()
  upload.mockResolvedValue({ id: 42 })
  submitted = {
    signingRequestId: 'request-1',
    signingRequestUrl: 'https://app.signpath.io/request-1',
  }
  statuses = [
    { status: 'WaitingForApproval', isFinalStatus: false },
    { status: 'Completed', isFinalStatus: true },
  ]
  approvalStatus = 204
  archive = zipSync({ 'app with spaces.exe': strToU8('signed bytes') })
  fetchMock.mockClear()
  const fakeFetch = async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = String(input)
    calls.push({ url, init })
    if (url.endsWith('/SigningRequests?api-version=1.0')) return Response.json(submitted)
    if (url.endsWith('/Status?api-version=1.0')) {
      return Response.json(statuses.length > 1 ? statuses.shift() : statuses[0])
    }
    if (url.endsWith('/Approve')) {
      return new Response(approvalStatus === 204 ? null : 'Approval is not permitted', {
        status: approvalStatus,
      })
    }
    if (url.endsWith('/SignedArtifact?api-version=1.0'))
      return new Response(new Uint8Array(archive).buffer)
    throw new Error(`Unexpected request: ${url}`)
  }
  fetchMock.mockImplementation(Object.assign(fakeFetch, { preconnect() {} }))
})

afterEach(async () => {
  for (const name of Object.keys(environment)) {
    if (originalEnvironment[name] === undefined) delete process.env[name]
    else process.env[name] = originalEnvironment[name]
  }
  if (!resolve(directory).startsWith(resolve(tmpdir()) + sep + 'koharu-signpath-test-')) {
    throw new Error('Unexpected fixture path')
  }
  await rm(directory, { recursive: true, force: true })
})

afterAll(() => {
  fetchMock.mockRestore()
  clock.mockRestore()
  logger.mockRestore()
})

test('uploads the input, uses connector origin metadata, approves, and replaces signed bytes', async () => {
  await signFile(file)
  expect(upload).toHaveBeenCalledWith(expect.stringMatching(/^signpath-/), [file], directory, {
    retentionDays: 3,
    compressionLevel: 0,
  })
  expect(JSON.parse(calls[0]!.init.body as string)).toEqual({
    artifactId: '42',
    gitHubWorkflowRunId: '123',
    gitHubRepository: 'koharu-rs/koharu',
    gitHubToken: 'github-token',
    signPathProjectSlug: 'koharu',
    signPathSigningPolicySlug: 'api-approval-policy',
    signPathArtifactConfigurationSlug: 'inline-file',
  })
  const approval = calls.find(({ url }) => url.endsWith('/Approve'))!
  expect(approval.url).toBe(
    'https://app.signpath.io/Api/v1/test-organization/SigningRequests/request-1/Approve',
  )
  expect(approval.init.method).toBe('POST')
  for (const call of calls) {
    expect(new Headers(call.init.headers).get('User-Agent')).toBe(
      `SignPath.SubmitSigningRequestGitHubAction/3.0.0(NodeJS/${process.version}; ${process.platform} ${process.arch}})`,
    )
    expect(new Headers(call.init.headers).get('Authorization')).toBe(
      call === approval ? 'Bearer approve-token' : 'Bearer submit-token',
    )
  }
  expect(await readFile(file, 'utf8')).toBe('signed bytes')
  expect(await readdir(directory)).toEqual(['app with spaces.exe'])
})

test('does not repeat an approval while a quorum is still pending', async () => {
  statuses.splice(1, 0, { status: 'WaitingForApproval', isFinalStatus: false })
  await signFile(file)
  expect(calls.filter(({ url }) => url.endsWith('/Approve'))).toHaveLength(1)
})

test.each(['dll', 'msi'])('signs a %s supplied by Tauri', async (extension) => {
  const name = `koharu.${extension}`
  const input = join(directory, name)
  await writeFile(input, 'unsigned bytes')
  archive = zipSync({ [name]: strToU8('signed bytes') })
  await signFile(input)
  expect(await readFile(input, 'utf8')).toBe('signed bytes')
  expect(await readFile(file, 'utf8')).toBe('unsigned bytes')
})

test('does not approve a request that is already completed', async () => {
  delete process.env.SIGNPATH_APPROVER_API_TOKEN
  statuses.shift()
  await signFile(file)
  expect(calls.some(({ url }) => url.endsWith('/Approve'))).toBe(false)
})

test('rejects origin validation failures without approval or file replacement', async () => {
  submitted = {
    validationResult: {
      errors: [{ error: 'Origin rejected', howToFix: 'Use an authorized build.' }],
    },
  }
  await expect(signFile(file)).rejects.toThrow('Origin rejected')
  expect(calls).toHaveLength(1)
  expect(await readFile(file, 'utf8')).toBe('unsigned bytes')
})

test('stops when API approval is forbidden', async () => {
  approvalStatus = 403
  await expect(signFile(file)).rejects.toThrow('SignPath HTTP 403')
  expect(calls.some(({ url }) => url.includes('/SignedArtifact'))).toBe(false)
  expect(await readFile(file, 'utf8')).toBe('unsigned bytes')
})

test.each(['Failed', 'Denied', 'Canceled'])(
  'preserves the input when signing ends as %s',
  async (status) => {
    statuses = [{ status, isFinalStatus: true }]
    await expect(signFile(file)).rejects.toThrow(`ended as ${status}`)
    expect(await readFile(file, 'utf8')).toBe('unsigned bytes')
  },
)

test('times out instead of leaving bundling waiting forever', async () => {
  statuses = [{ status: 'InProgress', isFinalStatus: false }]
  await expect(signFile(file)).rejects.toThrow('Timed out')
  expect(await readFile(file, 'utf8')).toBe('unsigned bytes')
})

test('rejects archives without the requested file', async () => {
  archive = zipSync({ 'other.exe': strToU8('wrong file') })
  await expect(signFile(file)).rejects.toThrow('does not contain')
  expect(await readFile(file, 'utf8')).toBe('unsigned bytes')
})

test('extracts only the exact requested archive entry', async () => {
  archive = zipSync({
    'app with spaces.exe': strToU8('signed bytes'),
    '../escape.exe': strToU8('unrelated'),
    'nested/extra.exe': strToU8('unrelated'),
  })
  await signFile(file)
  expect(await readdir(directory)).toEqual(['app with spaces.exe'])
})

test('waits for manual approval when no approver token is supplied', async () => {
  delete process.env.SIGNPATH_APPROVER_API_TOKEN
  await signFile(file)
  expect(calls.some(({ url }) => url.endsWith('/Approve'))).toBe(false)
})
