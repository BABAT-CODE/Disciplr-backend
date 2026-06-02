import { describe, it, expect, beforeEach, afterEach, jest } from '@jest/globals'
import type { CreateVaultInput, PersistedVault, StakeInput, StakeWithMemoInput } from '../types/vaults.js'
import { MemoTooLongError } from '../types/vaults.js'
import {
  buildVaultCreationPayload,
  buildVaultStakePayload,
  buildVaultStakeWithMemoPayload,
  getSorobanConfig,
  isSorobanSubmitEnabled,
  MEMO_MAX_BYTES,
  setSorobanClient,
  resetSorobanClient,
  type SorobanClient,
  type SorobanConfig,
} from '../services/soroban.js'

// ─── Fixtures ────────────────────────────────────────────────────────────────

const stellar = (): string => `G${'A'.repeat(55)}`

const makeInput = (overrides: Partial<CreateVaultInput> = {}): CreateVaultInput => ({
  amount: '1000',
  startDate: '2030-01-01T00:00:00.000Z',
  endDate: '2030-06-01T00:00:00.000Z',
  verifier: stellar(),
  destinations: { success: stellar(), failure: stellar() },
  milestones: [
    { title: 'Kickoff', dueDate: '2030-02-01T00:00:00.000Z', amount: '300' },
    { title: 'Final', dueDate: '2030-05-01T00:00:00.000Z', amount: '700' },
  ],
  ...overrides,
})

const makeVault = (overrides: Partial<PersistedVault> = {}): PersistedVault => ({
  id: 'vault-test-abc123',
  amount: '1000',
  startDate: '2030-01-01T00:00:00.000Z',
  endDate: '2030-06-01T00:00:00.000Z',
  verifier: stellar(),
  successDestination: stellar(),
  failureDestination: stellar(),
  creator: stellar(),
  status: 'draft',
  createdAt: '2025-03-25T00:00:00.000Z',
  milestones: [
    {
      id: 'ms-1',
      vaultId: 'vault-test-abc123',
      title: 'Kickoff',
      description: null,
      dueDate: '2030-02-01T00:00:00.000Z',
      amount: '300',
      sortOrder: 0,
      createdAt: '2025-03-25T00:00:00.000Z',
    },
    {
      id: 'ms-2',
      vaultId: 'vault-test-abc123',
      title: 'Final',
      description: null,
      dueDate: '2030-05-01T00:00:00.000Z',
      amount: '700',
      sortOrder: 1,
      createdAt: '2025-03-25T00:00:00.000Z',
    },
  ],
  ...overrides,
})

// ─── Env helpers ─────────────────────────────────────────────────────────────

const FULL_ENV = {
  SOROBAN_CONTRACT_ID: 'CABCDEF1234567890',
  SOROBAN_NETWORK_PASSPHRASE: 'Test SDF Network ; September 2015',
  SOROBAN_SOURCE_ACCOUNT: stellar(),
  SOROBAN_RPC_URL: 'https://soroban-testnet.stellar.org',
  SOROBAN_SECRET_KEY: 'SCZANGBA5YHTNYVVV3C7CAZMCLPVAR3LXKLHEADMPROMU3QAHZGOSN6A',
}

const savedEnv: Record<string, string | undefined> = {}

const setEnv = (vars: Record<string, string>): void => {
  for (const [key, value] of Object.entries(vars)) {
    savedEnv[key] = process.env[key]
    process.env[key] = value
  }
}

const clearSorobanEnv = (): void => {
  for (const key of Object.keys(FULL_ENV)) {
    savedEnv[key] = process.env[key]
    delete process.env[key]
  }
}

const restoreEnv = (): void => {
  for (const [key, value] of Object.entries(savedEnv)) {
    if (value === undefined) {
      delete process.env[key]
    } else {
      process.env[key] = value
    }
  }
}

// ─── Mock client factory ─────────────────────────────────────────────────────

const createMockClient = (
  result?: { txHash: string },
  error?: Error,
): {
  client: SorobanClient
  creationSpy: jest.Mock<SorobanClient['submitVaultCreation']>
  stakeSpy: jest.Mock<SorobanClient['submitStake']>
  memoSpy: jest.Mock<SorobanClient['submitStakeWithMemo']>
} => {
  const creationSpy = jest.fn<SorobanClient['submitVaultCreation']>()
  const stakeSpy = jest.fn<SorobanClient['submitStake']>()
  const memoSpy = jest.fn<SorobanClient['submitStakeWithMemo']>()
  if (error) {
    creationSpy.mockRejectedValue(error)
    stakeSpy.mockRejectedValue(error)
    memoSpy.mockRejectedValue(error)
  } else {
    const defaultTx = result ?? { txHash: 'mock-tx-hash-abc123' }
    creationSpy.mockResolvedValue(defaultTx)
    stakeSpy.mockResolvedValue(defaultTx)
    memoSpy.mockResolvedValue(defaultTx)
  }
  return {
    client: { submitVaultCreation: creationSpy, submitStake: stakeSpy, submitStakeWithMemo: memoSpy },
    creationSpy,
    stakeSpy,
    memoSpy,
  }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe('soroban service', () => {
  beforeEach(() => {
    clearSorobanEnv()
  })

  afterEach(() => {
    restoreEnv()
    resetSorobanClient()
  })

  // ─── getSorobanConfig ───────────────────────────────────────────

  describe('getSorobanConfig', () => {
    it('returns null when no env vars are set', () => {
      expect(getSorobanConfig()).toBeNull()
    })

    it('returns null when only some env vars are set', () => {
      setEnv({
        SOROBAN_CONTRACT_ID: 'CABCDEF',
        SOROBAN_RPC_URL: 'https://rpc.example.com',
      })
      expect(getSorobanConfig()).toBeNull()
    })

    it('returns config when all env vars are present', () => {
      setEnv(FULL_ENV)
      const config = getSorobanConfig()
      expect(config).not.toBeNull()
      expect(config!.contractId).toBe(FULL_ENV.SOROBAN_CONTRACT_ID)
      expect(config!.rpcUrl).toBe(FULL_ENV.SOROBAN_RPC_URL)
      expect(config!.secretKey).toBe(FULL_ENV.SOROBAN_SECRET_KEY)
    })
  })

  // ─── isSorobanSubmitEnabled ─────────────────────────────────────

  describe('isSorobanSubmitEnabled', () => {
    it('returns false when env is not configured', () => {
      expect(isSorobanSubmitEnabled()).toBe(false)
    })

    it('returns true when fully configured', () => {
      setEnv(FULL_ENV)
      expect(isSorobanSubmitEnabled()).toBe(true)
    })
  })

  // ─── buildVaultCreationPayload — build mode ─────────────────────

  describe('buildVaultCreationPayload (mode=build)', () => {
    it('returns not_requested submission when mode is build', async () => {
      const input = makeInput()
      const vault = makeVault()

      const result = await buildVaultCreationPayload(input, vault)

      expect(result.mode).toBe('build')
      expect(result.payload.method).toBe('create_vault')
      expect(result.submission.attempted).toBe(false)
      expect(result.submission.status).toBe('not_requested')
      expect(result.submission.txHash).toBeUndefined()
    })

    it('defaults to build mode when onChain is undefined', async () => {
      const input = makeInput({ onChain: undefined })
      const vault = makeVault()

      const result = await buildVaultCreationPayload(input, vault)

      expect(result.mode).toBe('build')
      expect(result.submission.status).toBe('not_requested')
    })

    it('includes vault args in payload', async () => {
      const vault = makeVault()
      const result = await buildVaultCreationPayload(makeInput(), vault)

      expect(result.payload.args.vaultId).toBe(vault.id)
      expect(result.payload.args.amount).toBe(vault.amount)
      expect(result.payload.args.verifier).toBe(vault.verifier)
      expect(result.payload.args.successDestination).toBe(vault.successDestination)
      expect(result.payload.args.failureDestination).toBe(vault.failureDestination)
    })

    it('maps milestones correctly', async () => {
      const vault = makeVault()
      const result = await buildVaultCreationPayload(makeInput(), vault)

      const milestones = result.payload.args.milestones as Array<Record<string, unknown>>
      expect(milestones).toHaveLength(2)
      expect(milestones[0]).toEqual({
        id: 'ms-1',
        title: 'Kickoff',
        amount: '300',
        dueDate: '2030-02-01T00:00:00.000Z',
      })
    })

    it('uses env-based contractId when input.onChain.contractId is absent', async () => {
      setEnv({ SOROBAN_CONTRACT_ID: 'ENV_CONTRACT_ID' })
      const result = await buildVaultCreationPayload(makeInput(), makeVault())
      expect(result.payload.contractId).toBe('ENV_CONTRACT_ID')
    })

    it('prefers input.onChain.contractId over env', async () => {
      setEnv({ SOROBAN_CONTRACT_ID: 'ENV_CONTRACT_ID' })
      const input = makeInput({ onChain: { mode: 'build', contractId: 'INPUT_CONTRACT' } })
      const result = await buildVaultCreationPayload(input, makeVault())
      expect(result.payload.contractId).toBe('INPUT_CONTRACT')
    })

    it('falls back to DEFAULT_CONTRACT_ID when nothing is configured', async () => {
      const result = await buildVaultCreationPayload(makeInput(), makeVault())
      expect(result.payload.contractId).toBe('CONTRACT_ID_NOT_CONFIGURED')
    })
  })

  // ─── buildVaultCreationPayload — submit mode, not configured ────

  describe('buildVaultCreationPayload (mode=submit, not configured)', () => {
    it('returns not_configured when env is incomplete', async () => {
      const input = makeInput({ onChain: { mode: 'submit' } })
      const vault = makeVault()

      const result = await buildVaultCreationPayload(input, vault)

      expect(result.mode).toBe('submit')
      expect(result.submission.attempted).toBe(true)
      expect(result.submission.status).toBe('not_configured')
      expect(result.submission.txHash).toBeUndefined()
    })

    it('still includes the full payload even when not configured', async () => {
      const input = makeInput({ onChain: { mode: 'submit' } })
      const vault = makeVault()

      const result = await buildVaultCreationPayload(input, vault)

      expect(result.payload.method).toBe('create_vault')
      expect(result.payload.args.vaultId).toBe(vault.id)
    })
  })

  // ─── buildVaultCreationPayload — submit mode, configured + mocked SDK ──

  describe('buildVaultCreationPayload (mode=submit, configured)', () => {
    beforeEach(() => {
      setEnv(FULL_ENV)
    })

    it('submits successfully and returns txHash', async () => {
      const expectedHash = 'tx-hash-from-soroban-network'
      const { client, creationSpy } = createMockClient({ txHash: expectedHash })
      setSorobanClient(client)

      const input = makeInput({ onChain: { mode: 'submit' } })
      const vault = makeVault()

      const result = await buildVaultCreationPayload(input, vault)

      expect(result.mode).toBe('submit')
      expect(result.submission.attempted).toBe(true)
      expect(result.submission.status).toBe('success')
      expect(result.submission.txHash).toBe(expectedHash)
      expect(result.submission.error).toBeUndefined()

      // Verify the mock client was called with the right config and args
      expect(creationSpy).toHaveBeenCalledTimes(1)
      const [passedConfig, passedArgs] = creationSpy.mock.calls[0] as [SorobanConfig, Record<string, any>]
      expect(passedConfig.contractId).toBe(FULL_ENV.SOROBAN_CONTRACT_ID)
      expect(passedConfig.secretKey).toBe(FULL_ENV.SOROBAN_SECRET_KEY)
      expect(passedArgs.vaultId).toBe(vault.id)
    })

    it('returns error status when submission fails', async () => {
      const { client } = createMockClient(undefined, new Error('RPC timeout'))
      setSorobanClient(client)

      const input = makeInput({ onChain: { mode: 'submit' } })
      const vault = makeVault()

      const result = await buildVaultCreationPayload(input, vault)

      expect(result.submission.attempted).toBe(true)
      expect(result.submission.status).toBe('error')
      expect(result.submission.error).toBe('RPC timeout')
      expect(result.submission.txHash).toBeUndefined()
    })

    it('handles non-Error thrown values gracefully', async () => {
      const spy = jest.fn<SorobanClient['submitVaultCreation']>().mockRejectedValue('string-error')
      setSorobanClient({ submitVaultCreation: spy })

      const input = makeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultCreationPayload(input, makeVault())

      expect(result.submission.status).toBe('error')
      expect(result.submission.error).toBe('Unknown submission error')
    })

    it('does not leak secret key or PII in the response', async () => {
      const { client } = createMockClient({ txHash: 'safe-hash' })
      setSorobanClient(client)

      const input = makeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultCreationPayload(input, makeVault())
      const serialized = JSON.stringify(result)

      expect(serialized).not.toContain(FULL_ENV.SOROBAN_SECRET_KEY)
      expect(serialized).not.toContain('SCZANGBA') // prefix of test secret
    })

    it('passes full config to the client including rpcUrl', async () => {
      const { client, creationSpy } = createMockClient()
      setSorobanClient(client)

      await buildVaultCreationPayload(
        makeInput({ onChain: { mode: 'submit' } }),
        makeVault(),
      )

      const [passedConfig] = creationSpy.mock.calls[0] as [SorobanConfig, any]
      expect(passedConfig.rpcUrl).toBe(FULL_ENV.SOROBAN_RPC_URL)
      expect(passedConfig.networkPassphrase).toBe(FULL_ENV.SOROBAN_NETWORK_PASSPHRASE)
    })
  })

  // ─── Idempotent client behaviour ───────────────────────────────

  describe('idempotent client behaviour', () => {
    beforeEach(() => {
      setEnv(FULL_ENV)
    })

    it('produces identical payload structure on repeated calls with same vault', async () => {
      const { client } = createMockClient({ txHash: 'hash-1' })
      setSorobanClient(client)

      const input = makeInput({ onChain: { mode: 'submit' } })
      const vault = makeVault()

      const result1 = await buildVaultCreationPayload(input, vault)
      const result2 = await buildVaultCreationPayload(input, vault)

      // Payload shape is always the same regardless of call count
      expect(result1.payload).toEqual(result2.payload)
      expect(result1.mode).toBe(result2.mode)
    })

    it('build mode calls never invoke the client', async () => {
      const { client, creationSpy } = createMockClient()
      setSorobanClient(client)

      const input = makeInput({ onChain: { mode: 'build' } })
      await buildVaultCreationPayload(input, makeVault())
      await buildVaultCreationPayload(input, makeVault())

      expect(creationSpy).not.toHaveBeenCalled()
    })
  })

  // ─── Structured logging ────────────────────────────────────────

  describe('logging', () => {
    beforeEach(() => {
      setEnv(FULL_ENV)
    })

    it('logs on submit start and success without PII', async () => {
      const logSpy = jest.spyOn(console, 'log').mockImplementation(() => {})
      const { client } = createMockClient({ txHash: 'logged-hash' })
      setSorobanClient(client)

      await buildVaultCreationPayload(
        makeInput({ onChain: { mode: 'submit' } }),
        makeVault(),
      )

      const calls = logSpy.mock.calls.map((c) => c[0] as string)
      const startLog = calls.find((c) => c.includes('soroban.submit_start'))
      const successLog = calls.find((c) => c.includes('soroban.submit_success'))

      expect(startLog).toBeDefined()
      expect(successLog).toBeDefined()
      expect(successLog).toContain('logged-hash')

      // Ensure no secret key leakage in logs
      for (const entry of calls) {
        expect(entry).not.toContain(FULL_ENV.SOROBAN_SECRET_KEY)
      }

      logSpy.mockRestore()
    })

    it('logs on submit error', async () => {
      const errorSpy = jest.spyOn(console, 'error').mockImplementation(() => {})
      const { client } = createMockClient(undefined, new Error('network failure'))
      setSorobanClient(client)

      await buildVaultCreationPayload(
        makeInput({ onChain: { mode: 'submit' } }),
        makeVault(),
      )

      const calls = errorSpy.mock.calls.map((c) => c[0] as string)
      const errorLog = calls.find((c) => c.includes('soroban.submit_error'))
      expect(errorLog).toBeDefined()
      expect(errorLog).toContain('network failure')

      errorSpy.mockRestore()
    })

    it('logs warning when submit attempted but not configured', async () => {
      clearSorobanEnv()
      const logSpy = jest.spyOn(console, 'log').mockImplementation(() => {})

      await buildVaultCreationPayload(
        makeInput({ onChain: { mode: 'submit' } }),
        makeVault(),
      )

      const calls = logSpy.mock.calls.map((c) => c[0] as string)
      // warn goes to console.log in our structured logger at warn level
      // Actually it goes to console.log for warn level
      const warnLog = calls.find((c) => c.includes('soroban.submit_not_configured'))
      expect(warnLog).toBeDefined()

      logSpy.mockRestore()
    })
  })

  // ─── Stake idempotency ──────────────────────────────────────────
  //
  // The contract's `stake` method must be idempotent at the service
  // layer: repeated calls with the same vault + user produce identical
  // payloads, build mode never invokes the client, and the client
  // receives consistent args on repeated submit calls.

  const USER_A = stellar()
  const USER_B = `G${'B'.repeat(55)}` // different address

  const MEMO_4_BYTES = 'deadbeef'                     // 4 bytes
  const MEMO_64_BYTES = 'ab'.repeat(64)               // exactly 64 bytes
  const MEMO_65_BYTES = 'ab'.repeat(65)               // 65 bytes — over limit
  const MEMO_PREFIXED = '0xdeadbeef'                  // with 0x prefix

  const makeStakeInput = (overrides: Partial<StakeInput> = {}): StakeInput => ({
    vaultId: 'vault-stake-id',
    amount: '500',
    user: USER_A,
    ...overrides,
  })

  const makeStakeWithMemoInput = (overrides: Partial<StakeWithMemoInput> = {}): StakeWithMemoInput => ({
    vaultId: 'vault-memo-id',
    amount: '750',
    user: USER_A,
    memo: MEMO_4_BYTES,
    ...overrides,
  })

  describe('buildVaultStakePayload (mode=build)', () => {
    it('returns not_requested submission when mode is build', async () => {
      const input = makeStakeInput()
      const result = await buildVaultStakePayload(input)

      expect(result.mode).toBe('build')
      expect(result.payload.method).toBe('stake')
      expect(result.submission.attempted).toBe(false)
      expect(result.submission.status).toBe('not_requested')
    })

    it('defaults to build mode when onChain is undefined', async () => {
      const input = makeStakeInput({ onChain: undefined })
      const result = await buildVaultStakePayload(input)

      expect(result.mode).toBe('build')
      expect(result.submission.status).toBe('not_requested')
    })

    it('includes stake args in payload', async () => {
      const input = makeStakeInput({ vaultId: 'vault-1', amount: '1000', user: USER_B })
      const result = await buildVaultStakePayload(input)

      expect(result.payload.args.vaultId).toBe('vault-1')
      expect(result.payload.args.amount).toBe('1000')
      expect(result.payload.args.user).toBe(USER_B)
    })
  })

  describe('buildVaultStakePayload (mode=submit, not configured)', () => {
    it('returns not_configured when env is incomplete', async () => {
      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakePayload(input)

      expect(result.mode).toBe('submit')
      expect(result.submission.attempted).toBe(true)
      expect(result.submission.status).toBe('not_configured')
    })

    it('still includes the full payload even when not configured', async () => {
      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakePayload(input)

      expect(result.payload.method).toBe('stake')
      expect(result.payload.args.vaultId).toBe(input.vaultId)
    })
  })

  describe('buildVaultStakePayload (mode=submit, configured)', () => {
    beforeEach(() => {
      setEnv(FULL_ENV)
    })

    it('submits successfully and returns txHash', async () => {
      const expectedHash = 'stake-tx-hash'
      const { client, stakeSpy } = createMockClient({ txHash: expectedHash })
      setSorobanClient(client)

      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakePayload(input)

      expect(result.mode).toBe('submit')
      expect(result.submission.status).toBe('success')
      expect(result.submission.txHash).toBe(expectedHash)

      expect(stakeSpy).toHaveBeenCalledTimes(1)
      const [passedConfig, passedArgs] = stakeSpy.mock.calls[0] as [SorobanConfig, Record<string, unknown>]
      expect(passedConfig.contractId).toBe(FULL_ENV.SOROBAN_CONTRACT_ID)
      expect(passedArgs.vaultId).toBe(input.vaultId)
      expect(passedArgs.amount).toBe(input.amount)
      expect(passedArgs.user).toBe(input.user)
    })

    it('returns error status when submission fails', async () => {
      const { client } = createMockClient(undefined, new Error('Stake timeout'))
      setSorobanClient(client)

      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakePayload(input)

      expect(result.submission.status).toBe('error')
      expect(result.submission.error).toBe('Stake timeout')
    })

    it('handles non-Error thrown values gracefully', async () => {
      const stakeSpy = jest.fn<SorobanClient['submitStake']>().mockRejectedValue('string-error')
      setSorobanClient({ submitVaultCreation: jest.fn() as any, submitStake: stakeSpy })

      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakePayload(input)

      expect(result.submission.status).toBe('error')
      expect(result.submission.error).toBe('Unknown submission error')
    })

    it('does not leak secret key or PII in the response', async () => {
      const { client } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakePayload(input)

      const serialized = JSON.stringify(result)
      expect(serialized).not.toContain(FULL_ENV.SOROBAN_SECRET_KEY)
    })

    it('passes full config to the client including rpcUrl', async () => {
      const { client, stakeSpy } = createMockClient()
      setSorobanClient(client)

      await buildVaultStakePayload(makeStakeInput({ onChain: { mode: 'submit' } }))

      const [passedConfig] = stakeSpy.mock.calls[0] as [SorobanConfig, any]
      expect(passedConfig.rpcUrl).toBe(FULL_ENV.SOROBAN_RPC_URL)
    })
  })

  // ─── Stake idempotent client behaviour ─────────────────────────

  describe('stake idempotent client behaviour', () => {
    beforeEach(() => {
      setEnv(FULL_ENV)
    })

    it('produces identical payload structure on repeated calls with same input', async () => {
      const { client } = createMockClient({ txHash: 'stake-hash' })
      setSorobanClient(client)

      const input = makeStakeInput({ onChain: { mode: 'submit' } })

      const result1 = await buildVaultStakePayload(input)
      const result2 = await buildVaultStakePayload(input)

      expect(result1.payload).toEqual(result2.payload)
      expect(result1.mode).toBe(result2.mode)
    })

    it('build mode calls never invoke the client', async () => {
      const { client, stakeSpy } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeInput({ onChain: { mode: 'build' } })
      await buildVaultStakePayload(input)
      await buildVaultStakePayload(input)

      expect(stakeSpy).not.toHaveBeenCalled()
    })

    it('submit mode calls the client exactly once per invocation', async () => {
      const { client, stakeSpy } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      await buildVaultStakePayload(input)
      await buildVaultStakePayload(input)

      // Each call triggers exactly one client invocation
      expect(stakeSpy).toHaveBeenCalledTimes(2)
    })

    it('passes the same args on repeated submit calls', async () => {
      const { client, stakeSpy } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeInput({ onChain: { mode: 'submit' } })
      await buildVaultStakePayload(input)
      await buildVaultStakePayload(input)

      const call1Args = stakeSpy.mock.calls[0][1] as Record<string, unknown>
      const call2Args = stakeSpy.mock.calls[1][1] as Record<string, unknown>
      expect(call1Args).toEqual(call2Args)
    })

    it('produces different payloads for different users on the same vault', async () => {
      const inputA = makeStakeInput({ vaultId: 'vault-1', user: USER_A })
      const inputB = makeStakeInput({ vaultId: 'vault-1', user: USER_B })

      const resultA = await buildVaultStakePayload(inputA)
      const resultB = await buildVaultStakePayload(inputB)

      expect(resultA.payload.args.user).toBe(USER_A)
      expect(resultB.payload.args.user).toBe(USER_B)
      // Different user => different payload (expected — not a bug)
      expect(resultA.payload.args).not.toEqual(resultB.payload.args)
    })
  })

  // ─── Stake with memo ────────────────────────────────────────────
  //
  // `stake_with_memo` extends `stake` with an optional hex-encoded
  // Bytes payload bound to the vault funding event for off-chain
  // correlation (e.g. tx idempotency key / analytics link).

  describe('buildVaultStakeWithMemoPayload (mode=build)', () => {
    it('returns not_requested submission when mode is build', async () => {
      const input = makeStakeWithMemoInput()
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.mode).toBe('build')
      expect(result.payload.method).toBe('stake_with_memo')
      expect(result.submission.attempted).toBe(false)
      expect(result.submission.status).toBe('not_requested')
    })

    it('defaults to build mode when onChain is undefined', async () => {
      const input = makeStakeWithMemoInput({ onChain: undefined })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.mode).toBe('build')
    })

    it('includes memo in payload args when provided', async () => {
      const input = makeStakeWithMemoInput({ memo: MEMO_4_BYTES })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.args.memo).toBe(MEMO_4_BYTES)
    })

    it('includes memo as undefined when not provided', async () => {
      const input = makeStakeWithMemoInput({ memo: undefined })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.args.memo).toBeUndefined()
    })

    it('accepts memo with 0x prefix', async () => {
      const input = makeStakeWithMemoInput({ memo: MEMO_PREFIXED })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.args.memo).toBe(MEMO_PREFIXED)
    })

    it('accepts memo at exactly MEMO_MAX_BYTES', async () => {
      const input = makeStakeWithMemoInput({ memo: MEMO_64_BYTES })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.args.memo).toBe(MEMO_64_BYTES)
    })

    it('rejects memo exceeding MEMO_MAX_BYTES', async () => {
      const input = makeStakeWithMemoInput({ memo: MEMO_65_BYTES })

      await expect(buildVaultStakeWithMemoPayload(input)).rejects.toThrow(MemoTooLongError)
    })

    it('rejects memo exceeding MEMO_MAX_BYTES with a descriptive message', async () => {
      const input = makeStakeWithMemoInput({ memo: MEMO_65_BYTES })

      await expect(buildVaultStakeWithMemoPayload(input)).rejects.toThrow(
        'Memo exceeds maximum length: 65 bytes > 64 bytes',
      )
    })

    it('includes stake args alongside memo', async () => {
      const input = makeStakeWithMemoInput({ vaultId: 'vault-99', amount: '999', user: USER_B })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.args.vaultId).toBe('vault-99')
      expect(result.payload.args.amount).toBe('999')
      expect(result.payload.args.user).toBe(USER_B)
      expect(result.payload.args.memo).toBe(MEMO_4_BYTES)
    })
  })

  describe('buildVaultStakeWithMemoPayload (mode=submit, not configured)', () => {
    it('returns not_configured when env is incomplete', async () => {
      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.mode).toBe('submit')
      expect(result.submission.status).toBe('not_configured')
    })

    it('still includes full payload when not configured', async () => {
      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.method).toBe('stake_with_memo')
      expect(result.payload.args.memo).toBe(MEMO_4_BYTES)
    })
  })

  describe('buildVaultStakeWithMemoPayload (mode=submit, configured)', () => {
    beforeEach(() => {
      setEnv(FULL_ENV)
    })

    it('submits successfully and returns txHash', async () => {
      const expectedHash = 'memo-tx-hash'
      const { client, memoSpy } = createMockClient({ txHash: expectedHash })
      setSorobanClient(client)

      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.mode).toBe('submit')
      expect(result.submission.status).toBe('success')
      expect(result.submission.txHash).toBe(expectedHash)

      expect(memoSpy).toHaveBeenCalledTimes(1)
      const [, passedArgs] = memoSpy.mock.calls[0] as [SorobanConfig, Record<string, unknown>]
      expect(passedArgs.memo).toBe(MEMO_4_BYTES)
    })

    it('passes memo through to client when provided', async () => {
      const { client, memoSpy } = createMockClient()
      setSorobanClient(client)

      await buildVaultStakeWithMemoPayload(
        makeStakeWithMemoInput({ onChain: { mode: 'submit' }, memo: 'cafebabe' }),
      )

      const [, passedArgs] = memoSpy.mock.calls[0] as [SorobanConfig, Record<string, unknown>]
      expect(passedArgs.memo).toBe('cafebabe')
    })

    it('passes memo as undefined to client when not provided', async () => {
      const { client, memoSpy } = createMockClient()
      setSorobanClient(client)

      await buildVaultStakeWithMemoPayload(
        makeStakeWithMemoInput({ onChain: { mode: 'submit' }, memo: undefined }),
      )

      const [, passedArgs] = memoSpy.mock.calls[0] as [SorobanConfig, Record<string, unknown>]
      expect(passedArgs.memo).toBeUndefined()
    })

    it('returns error status when submission fails', async () => {
      const { client } = createMockClient(undefined, new Error('Memo RPC error'))
      setSorobanClient(client)

      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.submission.status).toBe('error')
      expect(result.submission.error).toBe('Memo RPC error')
    })

    it('handles non-Error thrown values gracefully', async () => {
      const memoSpy = jest.fn<SorobanClient['submitStakeWithMemo']>().mockRejectedValue('string-error')
      setSorobanClient({ submitVaultCreation: jest.fn() as any, submitStake: jest.fn() as any, submitStakeWithMemo: memoSpy })

      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.submission.status).toBe('error')
      expect(result.submission.error).toBe('Unknown submission error')
    })

    it('does not leak secret key or PII in the response', async () => {
      const { client } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      const result = await buildVaultStakeWithMemoPayload(input)

      const serialized = JSON.stringify(result)
      expect(serialized).not.toContain(FULL_ENV.SOROBAN_SECRET_KEY)
    })
  })

  // ─── Stake with memo idempotent client behaviour ──────────────

  describe('stake with memo idempotent client behaviour', () => {
    beforeEach(() => {
      setEnv(FULL_ENV)
    })

    it('produces identical payload on repeated calls with same input', async () => {
      const { client } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      const result1 = await buildVaultStakeWithMemoPayload(input)
      const result2 = await buildVaultStakeWithMemoPayload(input)

      expect(result1.payload).toEqual(result2.payload)
    })

    it('build mode never invokes the client', async () => {
      const { client, memoSpy } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeWithMemoInput({ onChain: { mode: 'build' } })
      await buildVaultStakeWithMemoPayload(input)
      await buildVaultStakeWithMemoPayload(input)

      expect(memoSpy).not.toHaveBeenCalled()
    })

    it('submit mode calls the client exactly once per invocation', async () => {
      const { client, memoSpy } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      await buildVaultStakeWithMemoPayload(input)
      await buildVaultStakeWithMemoPayload(input)

      expect(memoSpy).toHaveBeenCalledTimes(2)
    })

    it('passes same args on repeated submit calls', async () => {
      const { client, memoSpy } = createMockClient()
      setSorobanClient(client)

      const input = makeStakeWithMemoInput({ onChain: { mode: 'submit' } })
      await buildVaultStakeWithMemoPayload(input)
      await buildVaultStakeWithMemoPayload(input)

      const call1 = memoSpy.mock.calls[0][1] as Record<string, unknown>
      const call2 = memoSpy.mock.calls[1][1] as Record<string, unknown>
      expect(call1).toEqual(call2)
    })

    it('produces different payloads for different memos', async () => {
      const inputA = makeStakeWithMemoInput({ memo: 'abcd', onChain: undefined })
      const inputB = makeStakeWithMemoInput({ memo: '1234', onChain: undefined })

      const resultA = await buildVaultStakeWithMemoPayload(inputA)
      const resultB = await buildVaultStakeWithMemoPayload(inputB)

      expect(resultA.payload.args.memo).toBe('abcd')
      expect(resultB.payload.args.memo).toBe('1234')
      expect(resultA.payload.args).not.toEqual(resultB.payload.args)
    })
  })

  // ─── Memo validation edge cases ──────────────────────────────

  describe('memo validation edge cases', () => {
    it('accepts empty string memo', async () => {
      const input = makeStakeWithMemoInput({ memo: '' })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.args.memo).toBe('')
    })

    it('rejects odd-length hex as valid (no padding required)', async () => {
      // 'a' is technically 0.5 bytes — hex decode will pad. The byte
      // count check divides by 2 so 'a' → 0.5 bytes, which is 0 < 64.
      const input = makeStakeWithMemoInput({ memo: 'a' })
      const result = await buildVaultStakeWithMemoPayload(input)

      expect(result.payload.args.memo).toBe('a')
    })

    it('throws typed error MemoTooLongError', async () => {
      const input = makeStakeWithMemoInput({ memo: MEMO_65_BYTES })

      try {
        await buildVaultStakeWithMemoPayload(input)
        expect('should have thrown').toBe('but did not')
      } catch (err) {
        expect(err).toBeInstanceOf(MemoTooLongError)
        expect((err as MemoTooLongError).name).toBe('MemoTooLongError')
      }
    })

    it('rejects memo with exactly MEMO_MAX_BYTES + 1 (boundary)', async () => {
      const input = makeStakeWithMemoInput({ memo: 'ab'.repeat(MEMO_MAX_BYTES + 1) })

      await expect(buildVaultStakeWithMemoPayload(input)).rejects.toThrow(MemoTooLongError)
    })

    it('accepts memo with exactly MEMO_MAX_BYTES (boundary)', async () => {
      const input = makeStakeWithMemoInput({ memo: 'ab'.repeat(MEMO_MAX_BYTES) })

      const result = await buildVaultStakeWithMemoPayload(input)
      expect(result.payload.args.memo).toBe('ab'.repeat(MEMO_MAX_BYTES))
    })
  })

  // ─── Edge cases ────────────────────────────────────────────────

  describe('edge cases', () => {
    it('handles vault with empty milestones array', async () => {
      const vault = makeVault({ milestones: [] })
      const result = await buildVaultCreationPayload(makeInput(), vault)

      const milestones = result.payload.args.milestones as unknown[]
      expect(milestones).toEqual([])
    })

    it('handles vault with null creator', async () => {
      const vault = makeVault({ creator: null })
      const result = await buildVaultCreationPayload(makeInput(), vault)

      expect(result.payload.args.vaultId).toBe(vault.id)
    })

    it('returns correct default networkPassphrase when env is not set', async () => {
      const result = await buildVaultCreationPayload(makeInput(), makeVault())
      expect(result.payload.networkPassphrase).toBe('Test SDF Network ; September 2015')
    })

    it('returns correct default sourceAccount when env is not set', async () => {
      const result = await buildVaultCreationPayload(makeInput(), makeVault())
      expect(result.payload.sourceAccount).toBe('SOURCE_ACCOUNT_NOT_CONFIGURED')
    })
  })
})
