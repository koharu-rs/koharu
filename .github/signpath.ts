// @ts-nocheck
import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { rename, writeFile } from 'node:fs/promises'
import { basename, dirname, resolve } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

import { DefaultArtifactClient } from '@actions/artifact'
import { unzipSync } from 'fflate'

// Protocol and User-Agent from SignPath/github-action-submit-signing-request@4184a38a17e5b37d026a3ed443581c1f03ae1b3a.
const userAgent = `SignPath.SubmitSigningRequestGitHubAction/3.0.0(NodeJS/${process.version}; ${process.platform} ${process.arch}})`
const connector = 'https://pipelineconnector.connectors.signpath.io/GitHubActions/GitHubCom'

async function request(url: string, token: string, init: RequestInit = {}) {
  const response = await fetch(url, {
    ...init,
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
      'User-Agent': userAgent,
    },
    signal: AbortSignal.timeout(300_000),
  })
  if (!response.ok) console.error(await response.text())
  assert(response.ok, `SignPath HTTP ${response.status}`)
  return response
}

export async function signFile(input: string) {
  const file = resolve(input)
  const name = basename(file)
  const token = process.env.SIGNPATH_API_TOKEN!
  const approver = process.env.SIGNPATH_APPROVER_API_TOKEN
  const organization = process.env.SIGNPATH_ORGANIZATION_ID!
  const requests = `${connector}/${organization}/SigningRequests`
  const { id } = await new DefaultArtifactClient().uploadArtifact(
    `signpath-${randomUUID()}`,
    [file],
    dirname(file),
    { retentionDays: 3, compressionLevel: 0 },
  )
  assert(id, 'GitHub did not return an artifact ID')
  const submission = (await (
    await request(`${requests}?api-version=1.0`, token, {
      method: 'POST',
      body: JSON.stringify({
        artifactId: String(id),
        gitHubWorkflowRunId: process.env.GITHUB_RUN_ID,
        gitHubRepository: process.env.GITHUB_REPOSITORY,
        gitHubToken: process.env.GITHUB_TOKEN,
        signPathProjectSlug: 'koharu',
        signPathSigningPolicySlug: process.env.SIGNPATH_SIGNING_POLICY_SLUG,
        signPathArtifactConfigurationSlug: process.env.SIGNPATH_ARTIFACT_CONFIGURATION_SLUG,
      }),
    })
  ).json()) as {
    signingRequestId?: string
    signingRequestUrl?: string
    validationResult?: { errors: unknown[] }
  }
  assert(
    submission.signingRequestId && !submission.validationResult?.errors.length,
    JSON.stringify(submission),
  )
  console.log(`SignPath request for ${name}: ${submission.signingRequestUrl}`)

  const requestId = submission.signingRequestId
  const deadline = Date.now() + 600_000
  let approved = false
  let status: { status: string; isFinalStatus: boolean }
  do {
    assert(Date.now() < deadline, `Timed out waiting for SignPath request ${requestId}`)
    status = (await (
      await request(`${requests}/${requestId}/Status?api-version=1.0`, token)
    ).json()) as typeof status
    // Foundation releases require manual approval. Supply an approver token only for policies permitting API approval.
    if (status.status === 'WaitingForApproval' && approver && !approved) {
      await request(
        `https://app.signpath.io/Api/v1/${organization}/SigningRequests/${requestId}/Approve`,
        approver,
        { method: 'POST' },
      )
      approved = true
      console.log(`SignPath request ${requestId} approved via API`)
    }
    if (!status.isFinalStatus) await sleep(10_000)
  } while (!status.isFinalStatus)
  assert.equal(status.status, 'Completed', `SignPath request ended as ${status.status}`)

  const response = await request(`${requests}/${requestId}/SignedArtifact?api-version=1.0`, token)
  const signed = unzipSync(new Uint8Array(await response.arrayBuffer()), {
    filter: (entry) => entry.name === name,
  })[name]
  assert(signed?.length, `Signed artifact does not contain ${name}`)
  const temporary = `${file}.signpath-${randomUUID()}`
  await writeFile(temporary, signed)
  await rename(temporary, file)
}

if (import.meta.main) await signFile(process.argv[2]!)
