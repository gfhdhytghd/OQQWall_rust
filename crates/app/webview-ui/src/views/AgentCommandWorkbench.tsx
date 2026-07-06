import { useEffect, useMemo, useRef, useState } from 'react'
import { Button, Input } from '@heroui/react'
import * as Blockly from 'scratch-blocks'
import * as ZhHans from 'blockly/msg/zh-hans'
import {
  Maximize2,
  MessageSquare,
  Minimize2,
  Minus,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  RotateCcw,
  Trash2,
} from 'lucide-react'
import {
  AgentCommandBlock,
  AgentCommandGlobalAction,
  AgentCommandPostTarget,
  AgentCommandQueueInsertPosition,
  AgentCommandReviewAction,
  AgentCommandShortcutScope,
  AgentCommandTrigger,
  AppConfigAgentCommand,
  AppConfigSettingsResponse,
  BlockKindFilter,
  BlockSelector,
  DraftTransform,
  IndexFilter,
  MediaKind,
  PositionSpec,
  RuleCondition,
  TextMatcher,
} from '../api/types'
import {
  AGENT_GLOBAL_ACTION_OPTIONS,
  AGENT_QUEUE_INSERT_POSITION_OPTIONS,
  AGENT_REVIEW_ACTION_OPTIONS,
  AGENT_SHORTCUT_SCOPE_OPTIONS,
  EmptyPanel,
  type AgentGlobalActionKey,
  type AgentReviewActionKey,
} from '../shared'

type AgentCommandVariable = AppConfigSettingsResponse['agent_command_variables'][number]

const ROOT_BLOCK_TYPE = 'oqqwall_agent_start'
const REPLY_BLOCK_TYPE = 'oqqwall_reply_private_message'
const SEND_WEBHOOK_BLOCK_TYPE = 'oqqwall_send_webhook'
const INSERT_QUEUE_BLOCK_TYPE = 'oqqwall_insert_queued_post'
const REVIEW_ACTION_BLOCK_TYPE = 'oqqwall_review_action'
const GLOBAL_ACTION_BLOCK_TYPE = 'oqqwall_global_action'
const IF_BLOCK_TYPE = 'oqqwall_if'
const SET_DRAFT_TRANSFORMS_BLOCK_TYPE = 'oqqwall_set_draft_transforms'
const MOVE_BLOCKS_BLOCK_TYPE = 'oqqwall_move_blocks'
const SELECTOR_BLOCK_TYPE = 'oqqwall_block_selector'
const POSITION_BLOCK_TYPE = 'oqqwall_position'
const CONDITION_HAS_BLOCK_TYPE = 'oqqwall_condition_has_block'
const CONDITION_COUNT_BLOCK_TYPE = 'oqqwall_condition_count'
const CONDITION_COMPOSITE_BLOCK_TYPE = 'oqqwall_condition_composite'
const CONDITION_NOT_BLOCK_TYPE = 'oqqwall_condition_not'
const VARIABLE_TOKEN_BLOCK_TYPE = 'oqqwall_variable_token'
const TEXT_LITERAL_BLOCK_TYPE = 'oqqwall_text_literal'
const JOIN_TEXT_BLOCK_TYPE = 'oqqwall_join_text'
const SCRATCH_BLOCKS_MEDIA_PATH = '/scratch-blocks-media/'

const AGENT_COMMAND_TRIGGER_OPTIONS: Array<{ value: AgentCommandTrigger; label: string }> = [
  { value: 'private_command', label: '私聊指令' },
  { value: 'submission_received', label: '收到新投稿' },
]

const BLOCK_KIND_SELECTION_OPTIONS = [
  { value: 'any', label: '任意块' },
  { value: 'paragraph', label: '段落' },
  { value: 'attachment_any', label: '附件' },
  { value: 'attachment_image', label: '图片' },
  { value: 'attachment_video', label: '视频' },
  { value: 'attachment_file', label: '文件' },
  { value: 'attachment_audio', label: '音频' },
  { value: 'attachment_sticker', label: '表情' },
  { value: 'reply', label: '回复引用' },
  { value: 'poke', label: '戳一戳' },
  { value: 'json_card', label: '卡片' },
  { value: 'forward', label: '合并转发' },
] as const

type BlockKindSelection = (typeof BLOCK_KIND_SELECTION_OPTIONS)[number]['value']

const TEXT_MATCHER_OPTIONS = [
  { value: 'none', label: '不匹配文本' },
  { value: 'contains', label: '包含' },
  { value: 'starts_with', label: '开头是' },
  { value: 'regex', label: '正则' },
] as const

type TextMatcherSelection = (typeof TEXT_MATCHER_OPTIONS)[number]['value']

const INDEX_FILTER_OPTIONS = [
  { value: 'all', label: '全部' },
  { value: 'first', label: '第一个' },
  { value: 'last', label: '最后一个' },
  { value: 'nth', label: '第 N 个' },
  { value: 'range', label: '范围' },
] as const

type IndexFilterSelection = (typeof INDEX_FILTER_OPTIONS)[number]['value']

const POSITION_OPTIONS = [
  { value: 'front', label: '最前' },
  { value: 'back', label: '最后' },
  { value: 'index', label: '第 N 位' },
  { value: 'before', label: '选择器之前' },
  { value: 'after', label: '选择器之后' },
] as const

type PositionSelection = (typeof POSITION_OPTIONS)[number]['value']

const POST_TARGET_OPTIONS = [
  { value: 'triggering_post', label: '当前触发稿件' },
  { value: 'review_code', label: '编号模板' },
] as const

type PostTargetSelection = (typeof POST_TARGET_OPTIONS)[number]['value']

const CONDITION_COMPOSITE_OPTIONS = [
  { value: 'all', label: '且' },
  { value: 'any', label: '或' },
] as const

const CONDITION_COUNT_OPTIONS = [
  { value: 'at_least', label: '≥' },
  { value: 'equals', label: '=' },
] as const

const SIMPLE_BLOCK_TYPES: Record<string, AgentCommandBlock['kind']> = {
  oqqwall_start_submission_session: 'start_submission_session',
  oqqwall_finish_submission_session: 'finish_submission_session',
  oqqwall_resume_submission_session: 'resume_submission_session',
  oqqwall_submit_submission_session: 'submit_submission_session',
  oqqwall_cancel_submission_session: 'cancel_submission_session',
}

const BLOCK_TYPE_BY_SIMPLE_KIND = Object.fromEntries(
  Object.entries(SIMPLE_BLOCK_TYPES).map(([type, kind]) => [kind, type])
) as Partial<Record<AgentCommandBlock['kind'], string>>

const scratchTheme = Blockly.Theme.defineTheme('oqqwall_scratch', {
  base: Blockly.Themes.Zelos,
  name: 'oqqwall_scratch',
  startHats: true,
  blockStyles: {
    event_blocks: {
      colourPrimary: '#ffbf00',
      colourSecondary: '#e6ac00',
      colourTertiary: '#cc9900',
    },
    message_blocks: {
      colourPrimary: '#4c97ff',
      colourSecondary: '#3373cc',
      colourTertiary: '#2f6bb7',
    },
    session_blocks: {
      colourPrimary: '#ff8c1a',
      colourSecondary: '#db6e00',
      colourTertiary: '#bf6000',
    },
    review_blocks: {
      colourPrimary: '#59c059',
      colourSecondary: '#389438',
      colourTertiary: '#2f7e2f',
    },
    system_blocks: {
      colourPrimary: '#9966ff',
      colourSecondary: '#774dcb',
      colourTertiary: '#5f3aa3',
    },
    rule_blocks: {
      colourPrimary: '#0fbd8c',
      colourSecondary: '#0b8f6a',
      colourTertiary: '#087654',
    },
    variable_blocks: {
      colourPrimary: '#ff6680',
      colourSecondary: '#d84f66',
      colourTertiary: '#b54155',
    },
  },
  categoryStyles: {
    event_category: { colour: '#ffbf00' },
    message_category: { colour: '#4c97ff' },
    session_category: { colour: '#ff8c1a' },
    review_category: { colour: '#59c059' },
    system_category: { colour: '#9966ff' },
    rule_category: { colour: '#0fbd8c' },
    variable_category: { colour: '#ff6680' },
  },
  componentStyles: {
    workspaceBackgroundColour: '#f7f9fb',
    toolboxBackgroundColour: '#ffffff',
    toolboxForegroundColour: '#161616',
    flyoutBackgroundColour: '#ffffff',
    flyoutForegroundColour: '#161616',
    flyoutOpacity: 1,
    scrollbarColour: '#8d98a8',
    insertionMarkerColour: '#0f62fe',
    selectedGlowColour: '#0f62fe',
    selectedGlowOpacity: 0.32,
  },
  fontStyle: {
    family:
      'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif',
    size: 12,
  },
})

let agentBlocksRegistered = false

export function AgentCommandWorkbench({
  commands,
  variables,
  onChange,
}: {
  commands: AppConfigAgentCommand[]
  variables: AgentCommandVariable[]
  onChange: (commands: AppConfigAgentCommand[]) => void
}) {
  const [selectedCommandIndex, setSelectedCommandIndex] = useState(0)
  const [isCommandRailCollapsed, setCommandRailCollapsed] = useState(false)
  const selectedCommand = commands[selectedCommandIndex] ?? null

  useEffect(() => {
    if (selectedCommandIndex >= commands.length) {
      setSelectedCommandIndex(Math.max(0, commands.length - 1))
    }
  }, [commands.length, selectedCommandIndex])

  function updateCommands(nextCommands: AppConfigAgentCommand[]) {
    onChange(nextCommands)
  }

  function updateCommand(
    commandIndex: number,
    updater: (command: AppConfigAgentCommand) => AppConfigAgentCommand
  ) {
    updateCommands(
      commands.map((command, index) => (index === commandIndex ? updater(command) : command))
    )
  }

  function addCommand() {
    const nextCommand = buildDefaultAgentCommand(buildNextAgentCommandName(commands))
    updateCommands([...commands, nextCommand])
    setSelectedCommandIndex(commands.length)
  }

  function removeCommand(commandIndex: number) {
    const nextCommands = commands.filter((_, index) => index !== commandIndex)
    updateCommands(nextCommands)
    setSelectedCommandIndex(Math.min(commandIndex, Math.max(0, nextCommands.length - 1)))
  }

  const selectedCommandKey = selectedCommand
    ? `command-${selectedCommandIndex}-${commands.length}`
    : 'empty'
  const sortedCommandTabs = commands
    .map((command, commandIndex) => ({ command, commandIndex }))
    .sort((left, right) =>
      normalizedAgentCommandName(left.command.name).localeCompare(
        normalizedAgentCommandName(right.command.name),
        'zh-Hans-CN'
      )
    )
  const selectedCommandHasDuplicateName =
    selectedCommand &&
    commands.some(
      (command, commandIndex) =>
        commandIndex !== selectedCommandIndex &&
        normalizedAgentCommandName(command.name) ===
          normalizedAgentCommandName(selectedCommand.name)
    )

  return (
    <div className="settings-panel scratch-workbench">
      <div className="settings-section-head">
        <div>
          <span className="field-label">Scratch Agent 指令</span>
          <p className="field-hint">
            每个触发词对应一个 Blockly 工作区，保存后立即生效。
          </p>
        </div>
        <div className="scratch-workbench-actions">
          <Button
            size="sm"
            variant="secondary"
            isIconOnly
            aria-label={isCommandRailCollapsed ? '展开指令列表' : '收起指令列表'}
            isDisabled={!commands.length}
            onClick={() => setCommandRailCollapsed((value) => !value)}
          >
            {isCommandRailCollapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          </Button>
          <Button size="sm" variant="secondary" onClick={addCommand}>
            <Plus size={16} />
            新增指令
          </Button>
        </div>
      </div>

      {commands.length ? (
        <div
          className={`scratch-command-layout${isCommandRailCollapsed ? ' is-rail-collapsed' : ''}`}
        >
          {isCommandRailCollapsed ? null : (
            <aside className="scratch-command-rail" aria-label="Agent 指令列表">
              {sortedCommandTabs.map(({ command, commandIndex }) => (
                <button
                  key={`${command.name}-${commandIndex}`}
                  type="button"
                  className={`scratch-command-tab${
                    commandIndex === selectedCommandIndex ? ' is-active' : ''
                  }${command.enabled ? '' : ' is-disabled'}`}
                  onClick={() => setSelectedCommandIndex(commandIndex)}
                >
                  <strong>#{command.name || `command_${commandIndex + 1}`}</strong>
                  <em>{agentCommandTriggerLabel(readAgentCommandTrigger(command.trigger))}</em>
                  <span>
                    {command.description ||
                      (command.enabled ? (command.admin_only ? '仅管理员' : '已启用') : '已关闭')}
                  </span>
                </button>
              ))}
            </aside>
          )}

          {selectedCommand ? (
            <section className="scratch-command-stage">
              <div className="scratch-command-meta">
                <div className="field-stack scratch-command-name-field">
                  <span className="field-label">触发词</span>
                  <Input
                    className="scratch-command-input"
                    placeholder="help"
                    value={selectedCommand.name}
                    onChange={(event) =>
                      updateCommand(selectedCommandIndex, (command) => ({
                        ...command,
                        name: event.target.value,
                      }))
                    }
                  />
                  {selectedCommandHasDuplicateName ? (
                    <span className="field-error">触发词与其他 Agent 指令重复。</span>
                  ) : null}
                </div>
                <div className="field-stack scratch-command-description-field">
                  <span className="field-label">说明</span>
                  <Input
                    className="scratch-command-input"
                    placeholder="给用户返回帮助菜单"
                    value={selectedCommand.description}
                    onChange={(event) =>
                      updateCommand(selectedCommandIndex, (command) => ({
                        ...command,
                        description: event.target.value,
                      }))
                    }
                  />
                </div>
                <div className="field-stack scratch-command-enable-field">
                  <button
                    type="button"
                    className={`scratch-command-enable-toggle${
                      selectedCommand.enabled ? ' is-enabled' : ''
                    }`}
                    aria-pressed={selectedCommand.enabled}
                    aria-label={selectedCommand.enabled ? '禁用当前指令' : '启用当前指令'}
                    onClick={() =>
                      updateCommand(selectedCommandIndex, (command) => ({
                        ...command,
                        enabled: !command.enabled,
                      }))
                    }
                  >
                    <span>{selectedCommand.enabled ? '启用' : '禁用'}</span>
                  </button>
                </div>
                <div className="field-stack scratch-command-enable-field">
                  <button
                    type="button"
                    className={`scratch-command-enable-toggle${
                      selectedCommand.admin_only ? ' is-enabled' : ''
                    }`}
                    aria-pressed={selectedCommand.admin_only}
                    aria-label={selectedCommand.admin_only ? '取消仅管理员' : '设为仅管理员'}
                    onClick={() =>
                      updateCommand(selectedCommandIndex, (command) => ({
                        ...command,
                        admin_only: !command.admin_only,
                      }))
                    }
                  >
                    <span>仅管理员</span>
                  </button>
                </div>
                <div className="scratch-command-actions">
                  <Button
                    size="sm"
                    variant="tertiary"
                    isIconOnly
                    aria-label="删除指令"
                    onClick={() => removeCommand(selectedCommandIndex)}
                  >
                    <Trash2 size={16} />
                  </Button>
                </div>
              </div>

              <ScratchBlocklyEditor
                key={selectedCommandKey}
                blocks={selectedCommand.blocks}
                trigger={readAgentCommandTrigger(selectedCommand.trigger)}
                variables={variables}
                onChange={(blocks, trigger) =>
                  updateCommand(selectedCommandIndex, (command) => ({ ...command, blocks, trigger }))
                }
              />
            </section>
          ) : null}
        </div>
      ) : (
        <EmptyPanel icon={<MessageSquare size={28} />} text="当前分组还没有配置用户 Agent 指令" />
      )}
    </div>
  )
}

function ScratchBlocklyEditor({
  blocks,
  trigger,
  variables,
  onChange,
}: {
  blocks: AgentCommandBlock[]
  trigger: AgentCommandTrigger
  variables: AgentCommandVariable[]
  onChange: (blocks: AgentCommandBlock[], trigger: AgentCommandTrigger) => void
}) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const workspaceRef = useRef<Blockly.WorkspaceSvg | null>(null)
  const onChangeRef = useRef(onChange)
  const initialBlocksRef = useRef(blocks)
  const initialTriggerRef = useRef(trigger)
  const [isFullscreen, setFullscreen] = useState(false)
  const [hasDetachedBlocks, setHasDetachedBlocks] = useState(false)
  const toolbox = useMemo(() => buildToolbox(variables), [variables])

  useEffect(() => {
    onChangeRef.current = onChange
  }, [onChange])

  useEffect(() => {
    registerAgentCommandBlocks()
    if (!containerRef.current) return

    const workspace = Blockly.inject(containerRef.current, {
      toolbox,
      scratchTheme: Blockly.ScratchBlocksTheme.CLASSIC,
      theme: scratchTheme,
      media: SCRATCH_BLOCKS_MEDIA_PATH,
      collapse: false,
      comments: false,
      disable: false,
      trashcan: true,
      sounds: false,
      grid: {
        spacing: 32,
        length: 3,
        colour: '#d0d7de',
        snap: true,
      },
      move: {
        scrollbars: true,
        drag: true,
        wheel: true,
      },
      zoom: {
        controls: false,
        wheel: true,
        pinch: true,
        startScale: 0.86,
        minScale: 0.55,
        maxScale: 1.35,
      },
    } as Parameters<typeof Blockly.inject>[1] & { media: string })
    workspaceRef.current = workspace

    loadAgentBlocksIntoWorkspace(workspace, initialBlocksRef.current, initialTriggerRef.current)
    setHasDetachedBlocks(workspaceHasDetachedAgentBlocks(workspace))

    const listener = (event: Blockly.Events.Abstract) => {
      if (event.isUiEvent) return
      const nextBlocks = workspaceToAgentBlocks(workspace)
      const nextTrigger = workspaceToAgentCommandTrigger(workspace)
      initialBlocksRef.current = nextBlocks
      initialTriggerRef.current = nextTrigger
      setHasDetachedBlocks(workspaceHasDetachedAgentBlocks(workspace))
      onChangeRef.current(nextBlocks, nextTrigger)
    }
    workspace.addChangeListener(listener)

    const resizeObserver = new ResizeObserver(() => Blockly.svgResize(workspace))
    resizeObserver.observe(containerRef.current)
    window.requestAnimationFrame(() => Blockly.svgResize(workspace))
    const cleanupDropdownCorrection = installBlocklyDropdownCorrection(containerRef.current)

    return () => {
      cleanupDropdownCorrection()
      resizeObserver.disconnect()
      workspace.removeChangeListener(listener)
      workspace.dispose()
      workspaceRef.current = null
    }
  }, [toolbox])

  useEffect(() => {
    const workspace = workspaceRef.current
    if (!workspace) return

    const resize = () => Blockly.svgResize(workspace)
    const animationFrame = window.requestAnimationFrame(resize)
    const timeout = window.setTimeout(resize, 180)
    return () => {
      window.cancelAnimationFrame(animationFrame)
      window.clearTimeout(timeout)
    }
  }, [isFullscreen])

  useEffect(() => {
    if (!isFullscreen) return undefined

    const previousOverflow = document.body.style.overflow
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setFullscreen(false)
    }

    document.body.style.overflow = 'hidden'
    window.addEventListener('keydown', onKeyDown)
    return () => {
      document.body.style.overflow = previousOverflow
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [isFullscreen])

  function zoomWorkspace(delta: -1 | 1) {
    const workspace = workspaceRef.current as
      | (Blockly.WorkspaceSvg & {
          markFocused?: () => void
          zoomCenter?: (delta: number) => void
        })
      | null
    workspace?.markFocused?.()
    workspace?.zoomCenter?.(delta)
  }

  function resetWorkspaceZoom() {
    const workspace = workspaceRef.current as
      | (Blockly.WorkspaceSvg & {
          markFocused?: () => void
          beginCanvasTransition?: () => void
          endCanvasTransition?: () => void
          zoomCenter?: (delta: number) => void
          scrollCenter?: () => void
          scale: number
          options: Blockly.WorkspaceSvg['options'] & {
            zoomOptions: {
              startScale: number
              scaleSpeed: number
            }
          }
        })
      | null
    if (!workspace) return

    workspace.markFocused?.()
    const startScale = workspace.options.zoomOptions.startScale
    const scaleSpeed = workspace.options.zoomOptions.scaleSpeed
    const zoomDelta = Math.log(startScale / workspace.scale) / Math.log(scaleSpeed)
    workspace.beginCanvasTransition?.()
    workspace.zoomCenter?.(zoomDelta)
    workspace.scrollCenter?.()
    window.setTimeout(() => workspace.endCanvasTransition?.(), 500)
  }

  return (
    <div className="scratch-blockly-shell">
      {hasDetachedBlocks ? (
        <p className="field-hint field-warning">
          未连接到起始帽的积木不会被保存和执行。
        </p>
      ) : null}
      <div className={`scratch-blockly-frame${isFullscreen ? ' is-fullscreen' : ''}`}>
        <div ref={containerRef} className="scratch-blockly-workspace" />
        <div className="scratch-blockly-frame-actions">
          <button
            type="button"
            className="scratch-blockly-button"
            aria-label={isFullscreen ? '退出页面内全屏' : '页面内全屏'}
            title={isFullscreen ? '退出页面内全屏' : '页面内全屏'}
            onClick={() => setFullscreen((value) => !value)}
          >
            {isFullscreen ? <Minimize2 size={18} /> : <Maximize2 size={18} />}
          </button>
        </div>
        <div className="scratch-blockly-zoom-controls" aria-label="缩放画布">
          <button
            type="button"
            className="scratch-blockly-button"
            aria-label="缩小"
            title="缩小"
            onClick={() => zoomWorkspace(-1)}
          >
            <Minus size={18} />
          </button>
          <button
            type="button"
            className="scratch-blockly-button"
            aria-label="复位缩放"
            title="复位缩放"
            onClick={resetWorkspaceZoom}
          >
            <RotateCcw size={18} />
          </button>
          <button
            type="button"
            className="scratch-blockly-button"
            aria-label="放大"
            title="放大"
            onClick={() => zoomWorkspace(1)}
          >
            <Plus size={18} />
          </button>
        </div>
      </div>
    </div>
  )
}

function installBlocklyDropdownCorrection(container: HTMLElement) {
  let animationFrame = 0
  let timeouts: number[] = []
  let lastPointer: { x: number; y: number } | null = null

  const clearTimeouts = () => {
    timeouts.forEach((timeout) => window.clearTimeout(timeout))
    timeouts = []
  }

  const schedule = () => {
    if (animationFrame) {
      window.cancelAnimationFrame(animationFrame)
    }
    animationFrame = window.requestAnimationFrame(() => {
      animationFrame = 0
      correctBlocklyDropdownPosition(lastPointer)
      clearTimeouts()
      timeouts = [0, 50, 150].map((delay) =>
        window.setTimeout(() => correctBlocklyDropdownPosition(lastPointer), delay)
      )
    })
  }

  const rememberPointer = (event: Event) => {
    if (event instanceof PointerEvent || event instanceof MouseEvent) {
      lastPointer = { x: event.clientX, y: event.clientY }
    }
    schedule()
  }

  const events = ['pointerdown', 'pointerup', 'click', 'keydown', 'focusin']
  events.forEach((eventName) => container.addEventListener(eventName, rememberPointer, true))
  window.addEventListener('scroll', schedule, true)
  window.addEventListener('resize', schedule)

  return () => {
    events.forEach((eventName) => container.removeEventListener(eventName, rememberPointer, true))
    window.removeEventListener('scroll', schedule, true)
    window.removeEventListener('resize', schedule)
    if (animationFrame) window.cancelAnimationFrame(animationFrame)
    clearTimeouts()
  }
}

function correctBlocklyDropdownPosition(anchor: { x: number; y: number } | null) {
  const dropdown = document.querySelector<HTMLElement>('.blocklyDropDownDiv')
  if (!dropdown) return

  const style = window.getComputedStyle(dropdown)
  if (style.display === 'none' || style.position !== 'fixed') return

  const top = Number.parseFloat(dropdown.style.top)
  const left = Number.parseFloat(dropdown.style.left)
  let nextTop = top
  let nextLeft = left

  if (Number.isFinite(top) && window.scrollY) {
    const viewportTop = top - window.scrollY
    const shouldRemoveScroll =
      (anchor && Math.abs(viewportTop - anchor.y) + 24 < Math.abs(top - anchor.y)) ||
      (!anchor && top > window.innerHeight && viewportTop < window.innerHeight)
    if (shouldRemoveScroll) nextTop = viewportTop
  }

  if (Number.isFinite(left) && window.scrollX) {
    const viewportLeft = left - window.scrollX
    const shouldRemoveScroll =
      (anchor && Math.abs(viewportLeft - anchor.x) + 24 < Math.abs(left - anchor.x)) ||
      (!anchor && left > window.innerWidth && viewportLeft < window.innerWidth)
    if (shouldRemoveScroll) nextLeft = viewportLeft
  }

  if (Number.isFinite(nextTop)) dropdown.style.top = `${nextTop}px`
  if (Number.isFinite(nextLeft)) dropdown.style.left = `${nextLeft}px`

  clampFixedElementToViewport(dropdown)
}

function clampFixedElementToViewport(element: HTMLElement) {
  const margin = 8
  const rect = element.getBoundingClientRect()
  let top = Number.parseFloat(element.style.top)
  let left = Number.parseFloat(element.style.left)

  if (Number.isFinite(top)) {
    if (rect.bottom > window.innerHeight - margin) {
      top -= rect.bottom - (window.innerHeight - margin)
    }
    if (rect.top < margin) {
      top += margin - rect.top
    }
    element.style.top = `${Math.max(margin, top)}px`
  }

  if (Number.isFinite(left)) {
    if (rect.right > window.innerWidth - margin) {
      left -= rect.right - (window.innerWidth - margin)
    }
    if (rect.left < margin) {
      left += margin - rect.left
    }
    element.style.left = `${Math.max(margin, left)}px`
  }
}

function registerAgentCommandBlocks() {
  if (agentBlocksRegistered || Object.prototype.hasOwnProperty.call(Blockly.Blocks, ROOT_BLOCK_TYPE)) {
    agentBlocksRegistered = true
    return
  }

  Blockly.setLocale(
    Object.fromEntries(Object.entries(ZhHans).filter(([key]) => key !== 'default')) as Record<
      string,
      string
    >
  )

  Blockly.defineBlocksWithJsonArray([
    {
      type: TEXT_LITERAL_BLOCK_TYPE,
      message0: '文本 %1',
      args0: [{ type: 'field_input', name: 'TEXT', text: '' }],
      output: 'String',
      style: 'message_blocks',
      tooltip: '输入固定文本；也可以把变量积木拖进文本槽替换它。',
    },
    {
      type: JOIN_TEXT_BLOCK_TYPE,
      message0: '拼接 %1 和 %2',
      args0: [textValueArg('LEFT'), textValueArg('RIGHT')],
      output: 'String',
      style: 'message_blocks',
      tooltip: '把两个文本或变量拼成一个模板。',
    },
    {
      type: ROOT_BLOCK_TYPE,
      message0: '当 %1 触发',
      args0: [
        {
          type: 'field_dropdown',
          name: 'TRIGGER',
          options: dropdownOptions(AGENT_COMMAND_TRIGGER_OPTIONS),
        },
      ],
      nextStatement: null,
      style: 'event_blocks',
      tooltip: '从这里开始串联这条 Agent 指令要执行的积木。',
    },
    {
      type: REPLY_BLOCK_TYPE,
      message0: '回复私聊',
      message1: '文案 %1',
      args1: [textValueArg('TEXT')],
      message2: '标签 %1',
      args2: [textValueArg('TAGS')],
      message3: '图片 %1',
      args3: [textValueArg('IMAGES')],
      previousStatement: null,
      nextStatement: null,
      style: 'message_blocks',
      tooltip: '向触发 Agent 指令的私聊用户发送消息。',
    },
    {
      type: SEND_WEBHOOK_BLOCK_TYPE,
      message0: '发送 Webhook',
      message1: '地址 %1',
      args1: [textValueArg('URL')],
      message2: 'source %1',
      args2: [textValueArg('SOURCE')],
      message3: '文本 %1',
      args3: [textValueArg('TEXT')],
      message4: '标签 %1',
      args4: [textValueArg('TAGS')],
      message5: '图片 %1',
      args5: [textValueArg('IMAGES')],
      previousStatement: null,
      nextStatement: null,
      style: 'message_blocks',
      tooltip: '把渲染后的文本、标签和图片发送到指定 Webhook。',
    },
    {
      type: 'oqqwall_start_submission_session',
      message0: '开始投稿会话',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
      tooltip: '开始一个投稿会话；会话进行中同样可触发 Agent 指令。',
    },
    {
      type: 'oqqwall_finish_submission_session',
      message0: '结束投稿会话并等待确认',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
      tooltip: '把投稿会话切到等待确认状态；会话进行中同样可触发 Agent 指令。',
    },
    {
      type: 'oqqwall_resume_submission_session',
      message0: '继续编辑投稿会话',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
      tooltip: '把等待确认的投稿会话切回继续编辑；会话进行中同样可触发 Agent 指令。',
    },
    {
      type: 'oqqwall_submit_submission_session',
      message0: '提交投稿会话',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
      tooltip: '提交等待确认的投稿会话；会话进行中同样可触发 Agent 指令。',
    },
    {
      type: 'oqqwall_cancel_submission_session',
      message0: '取消投稿会话',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
      tooltip: '取消当前投稿会话；会话进行中同样可触发 Agent 指令。',
    },
    {
      type: INSERT_QUEUE_BLOCK_TYPE,
      message0: '调整队列',
      message1: '移动稿件 %1',
      args1: [textValueArg('MOVING_CODE')],
      message2: '插入位置 %1',
      args2: [
        {
          type: 'field_dropdown',
          name: 'POSITION',
          options: dropdownOptions(AGENT_QUEUE_INSERT_POSITION_OPTIONS),
        },
      ],
      message3: '目标稿件 %1',
      args3: [textValueArg('ANCHOR_CODE')],
      previousStatement: null,
      nextStatement: null,
      style: 'system_blocks',
      tooltip: '把一个排队稿件移动到目标稿件之前或之后。',
    },
    {
      type: REVIEW_ACTION_BLOCK_TYPE,
      message0: '审核稿件 %1',
      args0: [textValueArg('REVIEW_CODE')],
      message1: '动作 %1',
      args1: [
        {
          type: 'field_dropdown',
          name: 'ACTION',
          options: dropdownOptions(AGENT_REVIEW_ACTION_OPTIONS),
        },
      ],
      message2: '参数 %1',
      args2: [textValueArg('ARG')],
      previousStatement: null,
      nextStatement: null,
      style: 'review_blocks',
      tooltip: '对指定审核编号执行一个审核动作；不需要参数的动作会忽略参数字段。',
    },
    {
      type: GLOBAL_ACTION_BLOCK_TYPE,
      message0: '系统动作 %1',
      args0: [
        {
          type: 'field_dropdown',
          name: 'ACTION',
          options: dropdownOptions(AGENT_GLOBAL_ACTION_OPTIONS),
        },
      ],
      message1: '参数 A %1',
      args1: [textValueArg('ARG_A')],
      message2: '参数 B %1',
      args2: [textValueArg('ARG_B')],
      message3: '快捷指令作用域 %1',
      args3: [
        {
          type: 'field_dropdown',
          name: 'SCOPE',
          options: dropdownOptions(AGENT_SHORTCUT_SCOPE_OPTIONS),
        },
      ],
      previousStatement: null,
      nextStatement: null,
      style: 'system_blocks',
      tooltip: '执行一个全局系统动作；不同动作会读取不同参数。',
    },
    {
      type: IF_BLOCK_TYPE,
      message0: '如果 %1',
      args0: [conditionValueArg('CONDITION')],
      message1: '那么 %1',
      args1: [{ type: 'input_statement', name: 'THEN' }],
      message2: '否则 %1',
      args2: [{ type: 'input_statement', name: 'ELSE' }],
      previousStatement: null,
      nextStatement: null,
      style: 'rule_blocks',
      tooltip: '根据当前触发稿件的内容块条件执行不同积木。',
    },
    {
      type: SET_DRAFT_TRANSFORMS_BLOCK_TYPE,
      message0: '设置 %1 的稿件变换',
      args0: [
        {
          type: 'field_dropdown',
          name: 'TARGET',
          options: dropdownOptions(POST_TARGET_OPTIONS),
        },
      ],
      message1: '编号模板 %1',
      args1: [textValueArg('REVIEW_CODE')],
      message2: '变换 %1',
      args2: [{ type: 'input_statement', name: 'TRANSFORMS', check: 'DraftTransform' }],
      previousStatement: null,
      nextStatement: null,
      style: 'rule_blocks',
      tooltip: '把一组内容块变换保存到指定待处理稿件，并触发重渲染。',
    },
    {
      type: MOVE_BLOCKS_BLOCK_TYPE,
      message0: '移动 %1 到 %2',
      args0: [selectorValueArg('SELECTOR'), positionValueArg('POSITION')],
      previousStatement: 'DraftTransform',
      nextStatement: 'DraftTransform',
      style: 'rule_blocks',
      tooltip: '按选择器找出内容块，保持相对顺序移动到目标位置。',
    },
    {
      type: CONDITION_HAS_BLOCK_TYPE,
      message0: '含 %1 块',
      args0: [selectorValueArg('SELECTOR')],
      output: 'RuleCondition',
      style: 'rule_blocks',
      tooltip: '判断当前稿件是否含有命中选择器的内容块。',
    },
    {
      type: CONDITION_COUNT_BLOCK_TYPE,
      message0: '%1 块数 %2 %3',
      args0: [
        selectorValueArg('SELECTOR'),
        {
          type: 'field_dropdown',
          name: 'OP',
          options: dropdownOptions(CONDITION_COUNT_OPTIONS),
        },
        { type: 'field_input', name: 'N', text: '1' },
      ],
      output: 'RuleCondition',
      style: 'rule_blocks',
      tooltip: '判断命中选择器的块数量。',
    },
    {
      type: CONDITION_COMPOSITE_BLOCK_TYPE,
      message0: '%1 %2 %3',
      args0: [
        conditionValueArg('LEFT'),
        {
          type: 'field_dropdown',
          name: 'OP',
          options: dropdownOptions(CONDITION_COMPOSITE_OPTIONS),
        },
        conditionValueArg('RIGHT'),
      ],
      output: 'RuleCondition',
      style: 'rule_blocks',
      tooltip: '组合两个条件。',
    },
    {
      type: CONDITION_NOT_BLOCK_TYPE,
      message0: '非 %1',
      args0: [conditionValueArg('CONDITION')],
      output: 'RuleCondition',
      style: 'rule_blocks',
      tooltip: '反转一个条件。',
    },
    {
      type: SELECTOR_BLOCK_TYPE,
      message0: '选择 %1',
      args0: [
        {
          type: 'field_dropdown',
          name: 'KIND',
          options: dropdownOptions(BLOCK_KIND_SELECTION_OPTIONS),
        },
      ],
      message1: '文本 %1 %2',
      args1: [
        {
          type: 'field_dropdown',
          name: 'TEXT_MODE',
          options: dropdownOptions(TEXT_MATCHER_OPTIONS),
        },
        { type: 'field_input', name: 'TEXT', text: '' },
      ],
      message2: '序号 %1 N %2 至 %3',
      args2: [
        {
          type: 'field_dropdown',
          name: 'INDEX_MODE',
          options: dropdownOptions(INDEX_FILTER_OPTIONS),
        },
        { type: 'field_input', name: 'N', text: '0' },
        { type: 'field_input', name: 'END', text: '0' },
      ],
      output: 'BlockSelector',
      style: 'rule_blocks',
      tooltip: '按块类型、段落文本和过滤后的序号选择内容块。',
    },
    {
      type: POSITION_BLOCK_TYPE,
      message0: '位置 %1 N %2 参照 %3',
      args0: [
        {
          type: 'field_dropdown',
          name: 'POS',
          options: dropdownOptions(POSITION_OPTIONS),
        },
        { type: 'field_input', name: 'N', text: '0' },
        selectorValueArg('SELECTOR'),
      ],
      output: 'PositionSpec',
      style: 'rule_blocks',
      tooltip: '指定移动后的插入位置；之前/之后需要参照选择器。',
    },
    {
      type: VARIABLE_TOKEN_BLOCK_TYPE,
      message0: '变量 %1',
      args0: [{ type: 'field_input', name: 'TOKEN', text: '<command_args>' }],
      output: 'String',
      style: 'variable_blocks',
      tooltip: '变量 token 可以复制到任意文本字段中使用。',
    },
  ])

  agentBlocksRegistered = true
}

function buildToolbox(variables: AgentCommandVariable[]) {
  return {
    kind: 'categoryToolbox',
    contents: [
      {
        kind: 'category',
        name: '消息',
        categorystyle: 'message_category',
        contents: [
          {
            kind: 'block',
            type: REPLY_BLOCK_TYPE,
            inputs: textInputShadows({ TEXT: '您好，<command_args>', TAGS: '', IMAGES: '' }),
          },
          {
            kind: 'block',
            type: SEND_WEBHOOK_BLOCK_TYPE,
            inputs: textInputShadows({
              URL: '',
              SOURCE: 'agent',
              TEXT: '<command_text>',
              TAGS: '',
              IMAGES: '',
            }),
          },
          {
            kind: 'block',
            type: TEXT_LITERAL_BLOCK_TYPE,
            fields: { TEXT: '文本' },
          },
          {
            kind: 'block',
            type: JOIN_TEXT_BLOCK_TYPE,
            inputs: textInputShadows({ LEFT: '', RIGHT: '' }),
          },
        ],
      },
      {
        kind: 'category',
        name: '投稿会话',
        categorystyle: 'session_category',
        contents: Object.keys(SIMPLE_BLOCK_TYPES).map((type) => ({ kind: 'block', type })),
      },
      {
        kind: 'category',
        name: '审核',
        categorystyle: 'review_category',
        contents: [
          {
            kind: 'block',
            type: REVIEW_ACTION_BLOCK_TYPE,
            fields: { ACTION: 'approve' },
            inputs: textInputShadows({
              REVIEW_CODE: '<previous_post_internal_code>',
              ARG: '',
            }),
          },
        ],
      },
      {
        kind: 'category',
        name: '系统',
        categorystyle: 'system_category',
        contents: [
          {
            kind: 'block',
            type: INSERT_QUEUE_BLOCK_TYPE,
            fields: {
              POSITION: 'before',
            },
            inputs: textInputShadows({
              MOVING_CODE: '<previous_post_internal_code>',
              ANCHOR_CODE: '',
            }),
          },
          {
            kind: 'block',
            type: GLOBAL_ACTION_BLOCK_TYPE,
            fields: { ACTION: 'help', SCOPE: 'review' },
            inputs: textInputShadows({ ARG_A: '', ARG_B: '' }),
          },
        ],
      },
      {
        kind: 'category',
        name: '规则',
        categorystyle: 'rule_category',
        contents: [
          {
            kind: 'block',
            type: IF_BLOCK_TYPE,
            inputs: {
              CONDITION: conditionInputShadow(),
            },
          },
          {
            kind: 'block',
            type: SET_DRAFT_TRANSFORMS_BLOCK_TYPE,
            fields: { TARGET: 'triggering_post' },
            inputs: {
              REVIEW_CODE: textInputShadow('<previous_post_internal_code>'),
              TRANSFORMS: {
                block: {
                  type: MOVE_BLOCKS_BLOCK_TYPE,
                  inputs: {
                    SELECTOR: selectorInputShadow('paragraph'),
                    POSITION: positionInputShadow('front'),
                  },
                },
              },
            },
          },
          {
            kind: 'block',
            type: MOVE_BLOCKS_BLOCK_TYPE,
            inputs: {
              SELECTOR: selectorInputShadow('paragraph'),
              POSITION: positionInputShadow('front'),
            },
          },
          {
            kind: 'block',
            type: CONDITION_COUNT_BLOCK_TYPE,
            fields: { OP: 'equals', N: '1' },
            inputs: {
              SELECTOR: selectorInputShadow('paragraph'),
            },
          },
          {
            kind: 'block',
            type: CONDITION_HAS_BLOCK_TYPE,
            inputs: {
              SELECTOR: selectorInputShadow('paragraph'),
            },
          },
          {
            kind: 'block',
            type: CONDITION_COMPOSITE_BLOCK_TYPE,
            fields: { OP: 'all' },
            inputs: {
              LEFT: conditionInputShadow(),
              RIGHT: conditionInputShadow(),
            },
          },
          {
            kind: 'block',
            type: CONDITION_NOT_BLOCK_TYPE,
            inputs: {
              CONDITION: conditionInputShadow(),
            },
          },
          {
            kind: 'block',
            type: SELECTOR_BLOCK_TYPE,
            fields: { KIND: 'paragraph', TEXT_MODE: 'none', INDEX_MODE: 'all' },
          },
          {
            kind: 'block',
            type: POSITION_BLOCK_TYPE,
            fields: { POS: 'front' },
            inputs: {
              SELECTOR: selectorInputShadow('paragraph'),
            },
          },
        ],
      },
      {
        kind: 'category',
        name: '变量',
        categorystyle: 'variable_category',
        contents: variables.length
          ? variables.map((variable) => ({
              kind: 'block',
              type: VARIABLE_TOKEN_BLOCK_TYPE,
              fields: { TOKEN: `<${variable.key}>` },
            }))
          : [{ kind: 'block', type: VARIABLE_TOKEN_BLOCK_TYPE }],
      },
    ],
  }
}

function loadAgentBlocksIntoWorkspace(
  workspace: Blockly.WorkspaceSvg,
  blocks: AgentCommandBlock[],
  trigger: AgentCommandTrigger
) {
  Blockly.Events.disable()
  try {
    workspace.clear()
    const root = workspace.newBlock(ROOT_BLOCK_TYPE)
    root.setDeletable(false)
    root.setMovable(false)
    root.initSvg()
    setBlocklyField(root, 'TRIGGER', trigger)
    root.render()
    root.moveBy(28, 28)

    connectAgentBlockStack(workspace, root.nextConnection, blocks)

    workspace.render()
  } finally {
    Blockly.Events.enable()
  }
}

function createBlocklyBlock(
  workspace: Blockly.WorkspaceSvg,
  agentBlock: AgentCommandBlock
): Blockly.Block | null {
  const type = blocklyTypeForAgentBlock(agentBlock)
  if (!type) return null
  const block = workspace.newBlock(type)
  block.initSvg()

  switch (agentBlock.kind) {
    case 'reply_private_message':
      setBlocklyTextInput(block, 'TEXT', agentBlock.text_template)
      setBlocklyTextInput(block, 'TAGS', joinListField(agentBlock.tags))
      setBlocklyTextInput(block, 'IMAGES', joinListField(agentBlock.images))
      break
    case 'send_webhook':
      setBlocklyTextInput(block, 'URL', agentBlock.url)
      setBlocklyTextInput(block, 'SOURCE', agentBlock.source_webhook)
      setBlocklyTextInput(block, 'TEXT', agentBlock.text_template)
      setBlocklyTextInput(block, 'TAGS', joinListField(agentBlock.tags))
      setBlocklyTextInput(block, 'IMAGES', joinListField(agentBlock.images))
      break
    case 'insert_queued_post':
      setBlocklyTextInput(block, 'MOVING_CODE', agentBlock.moving_post_code)
      setBlocklyTextInput(block, 'ANCHOR_CODE', agentBlock.anchor_post_code)
      setBlocklyField(block, 'POSITION', agentBlock.position)
      break
    case 'execute_review_action':
      setBlocklyTextInput(block, 'REVIEW_CODE', agentBlock.review_code)
      setBlocklyField(block, 'ACTION', agentBlock.action.action)
      setBlocklyTextInput(block, 'ARG', reviewActionArgument(agentBlock.action))
      break
    case 'execute_global_action':
      setBlocklyField(block, 'ACTION', agentBlock.action.action)
      setBlocklyTextInput(block, 'ARG_A', globalActionArgumentA(agentBlock.action))
      setBlocklyTextInput(block, 'ARG_B', globalActionArgumentB(agentBlock.action))
      setBlocklyField(block, 'SCOPE', globalActionScope(agentBlock.action))
      break
    case 'if':
      setBlocklyConditionInput(block, 'CONDITION', agentBlock.condition)
      connectAgentBlockStack(workspace, block.getInput('THEN')?.connection ?? null, agentBlock.then_blocks)
      connectAgentBlockStack(workspace, block.getInput('ELSE')?.connection ?? null, agentBlock.else_blocks)
      break
    case 'set_draft_transforms':
      setBlocklyPostTarget(block, agentBlock.target)
      connectDraftTransformStack(
        workspace,
        block.getInput('TRANSFORMS')?.connection ?? null,
        agentBlock.transforms
      )
      break
    default:
      break
  }

  block.render()
  return block
}

function createDraftTransformBlock(
  workspace: Blockly.Workspace,
  transform: DraftTransform
): Blockly.Block | null {
  if (transform.kind !== 'move_blocks') return null
  const block = workspace.newBlock(MOVE_BLOCKS_BLOCK_TYPE) as Blockly.BlockSvg
  block.initSvg()
  setBlocklySelectorInput(block, 'SELECTOR', transform.selector)
  setBlocklyPositionInput(block, 'POSITION', transform.position)
  block.render()
  return block
}

function createSelectorBlock(
  workspace: Blockly.Workspace,
  selector: BlockSelector
): Blockly.Block {
  const block = workspace.newBlock(SELECTOR_BLOCK_TYPE) as Blockly.BlockSvg
  block.initSvg()
  setBlocklyField(block, 'KIND', selectorKindSelection(selector))
  const text = selector.text ?? null
  setBlocklyField(block, 'TEXT_MODE', textMatcherSelection(text))
  setBlocklyField(block, 'TEXT', textMatcherValue(text))
  const index = selector.index ?? null
  setBlocklyField(block, 'INDEX_MODE', indexFilterSelection(index))
  setBlocklyField(block, 'N', indexFilterStart(index))
  setBlocklyField(block, 'END', indexFilterEnd(index))
  block.render()
  return block
}

function createPositionBlock(
  workspace: Blockly.Workspace,
  position: PositionSpec
): Blockly.Block {
  const block = workspace.newBlock(POSITION_BLOCK_TYPE) as Blockly.BlockSvg
  block.initSvg()
  setBlocklyField(block, 'POS', position.pos)
  setBlocklyField(block, 'N', position.pos === 'index' ? String(position.n) : '0')
  if (position.pos === 'before' || position.pos === 'after') {
    setBlocklySelectorInput(block, 'SELECTOR', position.selector)
  } else {
    setBlocklySelectorInput(block, 'SELECTOR', defaultBlockSelector('paragraph'))
  }
  block.render()
  return block
}

function createConditionBlock(
  workspace: Blockly.Workspace,
  condition: RuleCondition
): Blockly.Block {
  let block: Blockly.BlockSvg
  switch (condition.kind) {
    case 'all':
    case 'any': {
      block = workspace.newBlock(CONDITION_COMPOSITE_BLOCK_TYPE) as Blockly.BlockSvg
      block.initSvg()
      setBlocklyField(block, 'OP', condition.kind)
      const [left, right] = normalizeBinaryConditions(condition.kind, condition.conditions)
      setBlocklyConditionInput(block, 'LEFT', left)
      setBlocklyConditionInput(block, 'RIGHT', right)
      break
    }
    case 'not':
      block = workspace.newBlock(CONDITION_NOT_BLOCK_TYPE) as Blockly.BlockSvg
      block.initSvg()
      setBlocklyConditionInput(block, 'CONDITION', condition.condition)
      break
    case 'has_block':
      block = workspace.newBlock(CONDITION_HAS_BLOCK_TYPE) as Blockly.BlockSvg
      block.initSvg()
      setBlocklySelectorInput(block, 'SELECTOR', condition.selector)
      break
    case 'block_count_at_least':
    case 'block_count_equals':
      block = workspace.newBlock(CONDITION_COUNT_BLOCK_TYPE) as Blockly.BlockSvg
      block.initSvg()
      setBlocklyField(block, 'OP', condition.kind === 'block_count_at_least' ? 'at_least' : 'equals')
      setBlocklyField(block, 'N', String(condition.n))
      setBlocklySelectorInput(block, 'SELECTOR', condition.selector)
      break
  }
  block.render()
  return block
}

function connectAgentBlockStack(
  workspace: Blockly.WorkspaceSvg,
  connection: Blockly.Connection | null,
  blocks: AgentCommandBlock[]
) {
  let previousConnection = connection
  blocks.forEach((agentBlock) => {
    const block = createBlocklyBlock(workspace, agentBlock)
    if (!block?.previousConnection || !previousConnection) return
    previousConnection.connect(block.previousConnection)
    previousConnection = block.nextConnection
  })
}

function connectDraftTransformStack(
  workspace: Blockly.WorkspaceSvg,
  connection: Blockly.Connection | null,
  transforms: DraftTransform[]
) {
  let previousConnection = connection
  transforms.forEach((transform) => {
    const block = createDraftTransformBlock(workspace, transform)
    if (!block?.previousConnection || !previousConnection) return
    previousConnection.connect(block.previousConnection)
    previousConnection = block.nextConnection
  })
}

function workspaceToAgentBlocks(workspace: Blockly.WorkspaceSvg): AgentCommandBlock[] {
  const topBlocks = workspace.getTopBlocks(true)
  const root = topBlocks.find((block) => block.type === ROOT_BLOCK_TYPE)
  return root ? readAgentBlockStack(root.getNextBlock()) : []
}

function workspaceToAgentCommandTrigger(workspace: Blockly.WorkspaceSvg): AgentCommandTrigger {
  const root = workspace.getTopBlocks(true).find((block) => block.type === ROOT_BLOCK_TYPE)
  return root ? readAgentCommandTrigger(readBlocklyField(root, 'TRIGGER')) : 'private_command'
}

function workspaceHasDetachedAgentBlocks(workspace: Blockly.WorkspaceSvg) {
  return workspace
    .getTopBlocks(false)
    .some((block) => block.type !== ROOT_BLOCK_TYPE && isAgentStatementBlock(block))
}

function readAgentBlockStack(firstBlock: Blockly.Block | null): AgentCommandBlock[] {
  const blocks: AgentCommandBlock[] = []
  let currentBlock = firstBlock
  while (currentBlock) {
    const agentBlock = blocklyBlockToAgentBlock(currentBlock)
    if (agentBlock) blocks.push(agentBlock)
    currentBlock = currentBlock.getNextBlock()
  }
  return blocks
}

function readDraftTransformStack(firstBlock: Blockly.Block | null): DraftTransform[] {
  const transforms: DraftTransform[] = []
  let currentBlock = firstBlock
  while (currentBlock) {
    const transform = blocklyBlockToDraftTransform(currentBlock)
    if (transform) transforms.push(transform)
    currentBlock = currentBlock.getNextBlock()
  }
  return transforms
}

function blocklyBlockToAgentBlock(block: Blockly.Block): AgentCommandBlock | null {
  const simpleKind = SIMPLE_BLOCK_TYPES[block.type]
  if (simpleKind) return { kind: simpleKind } as AgentCommandBlock

  switch (block.type) {
    case REPLY_BLOCK_TYPE:
      return {
        kind: 'reply_private_message',
        text_template: readBlocklyTextInput(block, 'TEXT'),
        tags: parseListField(readBlocklyTextInput(block, 'TAGS')),
        images: parseListField(readBlocklyTextInput(block, 'IMAGES')),
      }
    case SEND_WEBHOOK_BLOCK_TYPE:
      return {
        kind: 'send_webhook',
        url: readBlocklyTextInput(block, 'URL').trim(),
        source_webhook: readBlocklyTextInput(block, 'SOURCE').trim(),
        text_template: readBlocklyTextInput(block, 'TEXT'),
        tags: parseListField(readBlocklyTextInput(block, 'TAGS')),
        images: parseListField(readBlocklyTextInput(block, 'IMAGES')),
      }
    case INSERT_QUEUE_BLOCK_TYPE:
      return {
        kind: 'insert_queued_post',
        moving_post_code: readBlocklyTextInput(block, 'MOVING_CODE').trim(),
        anchor_post_code: readBlocklyTextInput(block, 'ANCHOR_CODE').trim(),
        position: readQueueInsertPosition(readBlocklyField(block, 'POSITION')),
      }
    case REVIEW_ACTION_BLOCK_TYPE:
      return {
        kind: 'execute_review_action',
        review_code: readBlocklyTextInput(block, 'REVIEW_CODE').trim(),
        action: buildReviewActionFromBlockly(
          readReviewActionKey(readBlocklyField(block, 'ACTION')),
          readBlocklyTextInput(block, 'ARG')
        ),
      }
    case GLOBAL_ACTION_BLOCK_TYPE:
      return {
        kind: 'execute_global_action',
        action: buildGlobalActionFromBlockly(
          readGlobalActionKey(readBlocklyField(block, 'ACTION')),
          readBlocklyTextInput(block, 'ARG_A'),
          readBlocklyTextInput(block, 'ARG_B'),
          readShortcutScope(readBlocklyField(block, 'SCOPE'))
        ),
      }
    case IF_BLOCK_TYPE:
      return {
        kind: 'if',
        condition: readRuleConditionInput(block, 'CONDITION'),
        then_blocks: readAgentBlockStack(block.getInputTargetBlock('THEN')),
        else_blocks: readAgentBlockStack(block.getInputTargetBlock('ELSE')),
      }
    case SET_DRAFT_TRANSFORMS_BLOCK_TYPE:
      return {
        kind: 'set_draft_transforms',
        target: readAgentCommandPostTarget(block),
        transforms: readDraftTransformStack(block.getInputTargetBlock('TRANSFORMS')),
      }
    default:
      return null
  }
}

function blocklyBlockToDraftTransform(block: Blockly.Block): DraftTransform | null {
  switch (block.type) {
    case MOVE_BLOCKS_BLOCK_TYPE:
      return {
        kind: 'move_blocks',
        selector: readBlockSelectorInput(block, 'SELECTOR'),
        position: readPositionSpecInput(block, 'POSITION'),
      }
    default:
      return null
  }
}

function isAgentStatementBlock(block: Blockly.Block) {
  return (
    block.type in SIMPLE_BLOCK_TYPES ||
    [
      REPLY_BLOCK_TYPE,
      SEND_WEBHOOK_BLOCK_TYPE,
      INSERT_QUEUE_BLOCK_TYPE,
      REVIEW_ACTION_BLOCK_TYPE,
      GLOBAL_ACTION_BLOCK_TYPE,
      IF_BLOCK_TYPE,
      SET_DRAFT_TRANSFORMS_BLOCK_TYPE,
      MOVE_BLOCKS_BLOCK_TYPE,
    ].includes(block.type)
  )
}

function blocklyTypeForAgentBlock(block: AgentCommandBlock) {
  switch (block.kind) {
    case 'reply_private_message':
      return REPLY_BLOCK_TYPE
    case 'send_webhook':
      return SEND_WEBHOOK_BLOCK_TYPE
    case 'insert_queued_post':
      return INSERT_QUEUE_BLOCK_TYPE
    case 'execute_review_action':
      return REVIEW_ACTION_BLOCK_TYPE
    case 'execute_global_action':
      return GLOBAL_ACTION_BLOCK_TYPE
    case 'if':
      return IF_BLOCK_TYPE
    case 'set_draft_transforms':
      return SET_DRAFT_TRANSFORMS_BLOCK_TYPE
    default:
      return BLOCK_TYPE_BY_SIMPLE_KIND[block.kind] ?? null
  }
}

function buildReviewActionFromBlockly(
  action: AgentReviewActionKey,
  arg: string
): AgentCommandReviewAction {
  switch (action) {
    case 'defer':
      return { action, delay_ms: arg.trim() || '180000' }
    case 'comment':
    case 'reply':
      return { action, text_template: arg }
    case 'blacklist':
      return { action, reason_template: arg }
    case 'quick_reply':
      return { action, key_template: arg.trim() }
    case 'merge':
      return { action, target_review_code: arg.trim() }
    default:
      return buildDefaultReviewAction(action)
  }
}

function buildGlobalActionFromBlockly(
  action: AgentGlobalActionKey,
  argA: string,
  argB: string,
  scope: AgentCommandShortcutScope
): AgentCommandGlobalAction {
  const valueA = argA.trim()
  switch (action) {
    case 'recall':
    case 'withdraw':
    case 'info':
      return { action, review_code: valueA }
    case 'blacklist_add':
      return { action, sender_id: valueA, reason_template: argB }
    case 'blacklist_remove':
      return { action, sender_id: valueA }
    case 'set_external_number':
      return { action, value_template: valueA }
    case 'quick_reply_add':
      return { action, key_template: valueA, text_template: argB }
    case 'quick_reply_delete':
      return { action, key_template: valueA }
    case 'shortcut_add':
      return { action, scope, key_template: valueA, definition_template: argB }
    case 'shortcut_delete':
      return { action, scope, key_template: valueA }
    default:
      return buildDefaultGlobalAction(action)
  }
}

function reviewActionArgument(action: AgentCommandReviewAction) {
  switch (action.action) {
    case 'defer':
      return action.delay_ms
    case 'comment':
    case 'reply':
      return action.text_template
    case 'blacklist':
      return action.reason_template
    case 'quick_reply':
      return action.key_template
    case 'merge':
      return action.target_review_code
    default:
      return ''
  }
}

function globalActionArgumentA(action: AgentCommandGlobalAction) {
  switch (action.action) {
    case 'recall':
    case 'withdraw':
    case 'info':
      return action.review_code
    case 'blacklist_add':
    case 'blacklist_remove':
      return action.sender_id
    case 'set_external_number':
      return action.value_template
    case 'quick_reply_add':
    case 'quick_reply_delete':
    case 'shortcut_add':
    case 'shortcut_delete':
      return action.key_template
    default:
      return ''
  }
}

function globalActionArgumentB(action: AgentCommandGlobalAction) {
  switch (action.action) {
    case 'blacklist_add':
      return action.reason_template
    case 'quick_reply_add':
      return action.text_template
    case 'shortcut_add':
      return action.definition_template
    default:
      return ''
  }
}

function globalActionScope(action: AgentCommandGlobalAction): AgentCommandShortcutScope {
  switch (action.action) {
    case 'shortcut_add':
    case 'shortcut_delete':
      return action.scope
    default:
      return 'review'
  }
}

function buildDefaultAgentCommand(name: string): AppConfigAgentCommand {
  return {
    name,
    enabled: true,
    admin_only: false,
    trigger: 'private_command',
    description: '',
    blocks: [buildDefaultAgentCommandBlock('reply_private_message')],
  }
}

function buildDefaultReviewAction(action: AgentReviewActionKey): AgentCommandReviewAction {
  switch (action) {
    case 'approve':
    case 'reject':
    case 'delete':
    case 'skip':
    case 'immediate':
    case 'refresh':
    case 'rerender':
    case 'select_all_messages':
    case 'toggle_anonymous':
    case 'expand_audit':
    case 'show':
      return { action }
    case 'defer':
      return { action, delay_ms: '180000' }
    case 'comment':
    case 'reply':
      return { action, text_template: '' }
    case 'blacklist':
      return { action, reason_template: '' }
    case 'quick_reply':
      return { action, key_template: '' }
    case 'merge':
      return { action, target_review_code: '' }
  }
}

function buildDefaultGlobalAction(action: AgentGlobalActionKey): AgentCommandGlobalAction {
  switch (action) {
    case 'help':
    case 'manual_relogin':
    case 'auto_relogin':
    case 'pending_list':
    case 'pending_clear':
    case 'send_queue_clear':
    case 'send_queue_flush':
    case 'send_in_flight_clear':
    case 'blacklist_list':
    case 'quick_reply_list':
    case 'shortcut_list':
    case 'self_check':
    case 'system_repair':
      return { action }
    case 'recall':
    case 'withdraw':
    case 'info':
      return { action, review_code: '' }
    case 'blacklist_add':
      return { action, sender_id: '', reason_template: '' }
    case 'blacklist_remove':
      return { action, sender_id: '' }
    case 'set_external_number':
      return { action, value_template: '' }
    case 'quick_reply_add':
      return { action, key_template: '', text_template: '' }
    case 'quick_reply_delete':
      return { action, key_template: '' }
    case 'shortcut_add':
      return { action, scope: 'review', key_template: '', definition_template: '' }
    case 'shortcut_delete':
      return { action, scope: 'review', key_template: '' }
  }
}

function buildDefaultAgentCommandBlock(
  kind: AgentCommandBlock['kind']
): AgentCommandBlock {
  switch (kind) {
    case 'reply_private_message':
      return {
        kind,
        text_template: '',
        tags: [],
        images: [],
      }
    case 'start_submission_session':
    case 'finish_submission_session':
    case 'resume_submission_session':
    case 'submit_submission_session':
    case 'cancel_submission_session':
      return { kind }
    case 'insert_queued_post':
      return {
        kind,
        moving_post_code: '',
        anchor_post_code: '',
        position: 'before',
      }
    case 'execute_review_action':
      return {
        kind,
        review_code: '',
        action: buildDefaultReviewAction('approve'),
      }
    case 'execute_global_action':
      return {
        kind,
        action: buildDefaultGlobalAction('help'),
      }
    case 'if':
      return {
        kind,
        condition: defaultRuleCondition(),
        then_blocks: [],
        else_blocks: [],
      }
    case 'set_draft_transforms':
      return {
        kind,
        target: { target: 'triggering_post' },
        transforms: [defaultDraftTransform()],
      }
    case 'send_webhook':
      return {
        kind,
        url: '',
        source_webhook: '',
        text_template: '',
        tags: [],
        images: [],
      }
  }
}

function buildNextAgentCommandName(commands: AppConfigAgentCommand[]) {
  let index = commands.length + 1
  while (commands.some((command) => command.name === `command_${index}`)) {
    index += 1
  }
  return `command_${index}`
}

function normalizedAgentCommandName(name: string) {
  return name.trim().replace(/^#+/, '').trim()
}

function readReviewActionKey(value: string): AgentReviewActionKey {
  return AGENT_REVIEW_ACTION_OPTIONS.some((option) => option.value === value)
    ? (value as AgentReviewActionKey)
    : 'approve'
}

function readGlobalActionKey(value: string): AgentGlobalActionKey {
  return AGENT_GLOBAL_ACTION_OPTIONS.some((option) => option.value === value)
    ? (value as AgentGlobalActionKey)
    : 'help'
}

function readQueueInsertPosition(value: string): AgentCommandQueueInsertPosition {
  return value === 'after' ? 'after' : 'before'
}

function readShortcutScope(value: string): AgentCommandShortcutScope {
  return value === 'global' ? 'global' : 'review'
}

function readAgentCommandTrigger(value: string | undefined): AgentCommandTrigger {
  return value === 'submission_received' ? 'submission_received' : 'private_command'
}

function agentCommandTriggerLabel(trigger: AgentCommandTrigger) {
  return trigger === 'submission_received' ? '收到新投稿' : '私聊指令'
}

function readAgentCommandPostTarget(block: Blockly.Block): AgentCommandPostTarget {
  const target = readBlocklyField(block, 'TARGET')
  if (target === 'review_code') {
    return {
      target: 'review_code',
      template: readBlocklyTextInput(block, 'REVIEW_CODE').trim(),
    }
  }
  return { target: 'triggering_post' }
}

function setBlocklyPostTarget(block: Blockly.Block, target: AgentCommandPostTarget) {
  if (target.target === 'review_code') {
    setBlocklyField(block, 'TARGET', 'review_code')
    setBlocklyTextInput(block, 'REVIEW_CODE', target.template)
  } else {
    setBlocklyField(block, 'TARGET', 'triggering_post')
    setBlocklyTextInput(block, 'REVIEW_CODE', '')
  }
}

function readRuleConditionInput(block: Blockly.Block, inputName: string): RuleCondition {
  const targetBlock = block.getInputTargetBlock(inputName)
  return targetBlock ? blocklyBlockToRuleCondition(targetBlock) : defaultRuleCondition()
}

function blocklyBlockToRuleCondition(block: Blockly.Block): RuleCondition {
  switch (block.type) {
    case CONDITION_HAS_BLOCK_TYPE:
      return {
        kind: 'has_block',
        selector: readBlockSelectorInput(block, 'SELECTOR'),
      }
    case CONDITION_COUNT_BLOCK_TYPE: {
      const selector = readBlockSelectorInput(block, 'SELECTOR')
      const n = readIntegerField(block, 'N', 1)
      return readBlocklyField(block, 'OP') === 'at_least'
        ? { kind: 'block_count_at_least', selector, n }
        : { kind: 'block_count_equals', selector, n }
    }
    case CONDITION_COMPOSITE_BLOCK_TYPE: {
      const conditions = [
        readRuleConditionInput(block, 'LEFT'),
        readRuleConditionInput(block, 'RIGHT'),
      ]
      return readBlocklyField(block, 'OP') === 'any'
        ? { kind: 'any', conditions }
        : { kind: 'all', conditions }
    }
    case CONDITION_NOT_BLOCK_TYPE:
      return {
        kind: 'not',
        condition: readRuleConditionInput(block, 'CONDITION'),
      }
    default:
      return defaultRuleCondition()
  }
}

function readBlockSelectorInput(block: Blockly.Block, inputName: string): BlockSelector {
  const targetBlock = block.getInputTargetBlock(inputName)
  return targetBlock ? blocklyBlockToBlockSelector(targetBlock) : defaultBlockSelector('paragraph')
}

function blocklyBlockToBlockSelector(block: Blockly.Block): BlockSelector {
  if (block.type !== SELECTOR_BLOCK_TYPE) return defaultBlockSelector('paragraph')

  const selector: BlockSelector = {}
  const kinds = blockKindSelectionToKinds(readBlocklyField(block, 'KIND') as BlockKindSelection)
  if (kinds) selector.kinds = kinds

  const text = readTextMatcher(block)
  if (text) selector.text = text

  const index = readIndexFilter(block)
  if (index) selector.index = index

  return selector
}

function readPositionSpecInput(block: Blockly.Block, inputName: string): PositionSpec {
  const targetBlock = block.getInputTargetBlock(inputName)
  return targetBlock ? blocklyBlockToPositionSpec(targetBlock) : { pos: 'front' }
}

function blocklyBlockToPositionSpec(block: Blockly.Block): PositionSpec {
  if (block.type !== POSITION_BLOCK_TYPE) return { pos: 'front' }

  switch (readBlocklyField(block, 'POS') as PositionSelection) {
    case 'back':
      return { pos: 'back' }
    case 'index':
      return { pos: 'index', n: readIntegerField(block, 'N', 0) }
    case 'before':
      return {
        pos: 'before',
        selector: readBlockSelectorInput(block, 'SELECTOR'),
      }
    case 'after':
      return {
        pos: 'after',
        selector: readBlockSelectorInput(block, 'SELECTOR'),
      }
    default:
      return { pos: 'front' }
  }
}

function readTextMatcher(block: Blockly.Block): TextMatcher | null {
  const mode = readBlocklyField(block, 'TEXT_MODE') as TextMatcherSelection
  const value = readBlocklyField(block, 'TEXT')
  switch (mode) {
    case 'contains':
      clearBlocklyWarning(block)
      return { mode, needle: value }
    case 'starts_with':
      clearBlocklyWarning(block)
      return { mode, prefix: value }
    case 'regex':
      setRegexWarning(block, value)
      return { mode, pattern: value }
    default:
      clearBlocklyWarning(block)
      return null
  }
}

function readIndexFilter(block: Blockly.Block): IndexFilter | null {
  switch (readBlocklyField(block, 'INDEX_MODE') as IndexFilterSelection) {
    case 'first':
      return { mode: 'first' }
    case 'last':
      return { mode: 'last' }
    case 'nth':
      return { mode: 'nth', n: readIntegerField(block, 'N', 0) }
    case 'range':
      return {
        mode: 'range',
        start: readIntegerField(block, 'N', 0),
        end: readIntegerField(block, 'END', 0),
      }
    default:
      return null
  }
}

function blockKindSelectionToKinds(value: BlockKindSelection): BlockKindFilter[] | null {
  switch (value) {
    case 'paragraph':
    case 'reply':
    case 'poke':
    case 'json_card':
    case 'forward':
      return [value]
    case 'attachment_any':
      return [{ attachment: { media_kind: null } }]
    case 'attachment_image':
      return [{ attachment: { media_kind: 'Image' } }]
    case 'attachment_video':
      return [{ attachment: { media_kind: 'Video' } }]
    case 'attachment_file':
      return [{ attachment: { media_kind: 'File' } }]
    case 'attachment_audio':
      return [{ attachment: { media_kind: 'Audio' } }]
    case 'attachment_sticker':
      return [{ attachment: { media_kind: 'Sticker' } }]
    default:
      return null
  }
}

function selectorKindSelection(selector: BlockSelector): BlockKindSelection {
  const firstKind = selector.kinds?.[0]
  if (!firstKind) return 'any'
  if (typeof firstKind === 'string') return firstKind as BlockKindSelection
  const mediaKind = firstKind.attachment.media_kind
  switch (mediaKind) {
    case 'Image':
      return 'attachment_image'
    case 'Video':
      return 'attachment_video'
    case 'File':
      return 'attachment_file'
    case 'Audio':
      return 'attachment_audio'
    case 'Sticker':
      return 'attachment_sticker'
    default:
      return 'attachment_any'
  }
}

function textMatcherSelection(matcher: TextMatcher | null): TextMatcherSelection {
  return matcher?.mode ?? 'none'
}

function textMatcherValue(matcher: TextMatcher | null) {
  switch (matcher?.mode) {
    case 'contains':
      return matcher.needle
    case 'starts_with':
      return matcher.prefix
    case 'regex':
      return matcher.pattern
    default:
      return ''
  }
}

function indexFilterSelection(index: IndexFilter | null): IndexFilterSelection {
  return index?.mode ?? 'all'
}

function indexFilterStart(index: IndexFilter | null) {
  switch (index?.mode) {
    case 'nth':
      return String(index.n)
    case 'range':
      return String(index.start)
    default:
      return '0'
  }
}

function indexFilterEnd(index: IndexFilter | null) {
  return index?.mode === 'range' ? String(index.end) : '0'
}

function normalizeBinaryConditions(
  kind: 'all' | 'any',
  conditions: RuleCondition[]
): [RuleCondition, RuleCondition] {
  if (conditions.length === 0) return [defaultRuleCondition(), defaultRuleCondition()]
  if (conditions.length === 1) return [conditions[0], defaultRuleCondition()]
  if (conditions.length === 2) return [conditions[0], conditions[1]]
  return [conditions[0], { kind, conditions: conditions.slice(1) }]
}

function defaultRuleCondition(): RuleCondition {
  return {
    kind: 'block_count_equals',
    selector: defaultBlockSelector('paragraph'),
    n: 1,
  }
}

function defaultBlockSelector(kind: BlockKindSelection): BlockSelector {
  const selector: BlockSelector = {}
  const kinds = blockKindSelectionToKinds(kind)
  if (kinds) selector.kinds = kinds
  return selector
}

function defaultDraftTransform(): DraftTransform {
  return {
    kind: 'move_blocks',
    selector: defaultBlockSelector('paragraph'),
    position: { pos: 'front' },
  }
}

function readIntegerField(block: Blockly.Block, fieldName: string, fallback: number) {
  const parsed = Number.parseInt(readBlocklyField(block, fieldName), 10)
  return Number.isFinite(parsed) ? parsed : fallback
}

function setRegexWarning(block: Blockly.Block, pattern: string) {
  try {
    new RegExp(pattern)
    clearBlocklyWarning(block)
  } catch (error) {
    const message = error instanceof Error ? error.message : '正则表达式无效'
    setBlocklyWarning(block, `正则表达式无效：${message}`)
  }
}

function clearBlocklyWarning(block: Blockly.Block) {
  setBlocklyWarning(block, null)
}

function setBlocklyWarning(block: Blockly.Block, warning: string | null) {
  ;(block as Blockly.Block & { setWarningText?: (text: string | null) => void }).setWarningText?.(
    warning
  )
}

function dropdownOptions<T extends string>(options: ReadonlyArray<{ value: T; label: string }>) {
  return options.map((option) => [option.label, option.value])
}

function textValueArg(name: string) {
  return { type: 'input_value', name, check: 'String' }
}

function selectorValueArg(name: string) {
  return { type: 'input_value', name, check: 'BlockSelector' }
}

function positionValueArg(name: string) {
  return { type: 'input_value', name, check: 'PositionSpec' }
}

function conditionValueArg(name: string) {
  return { type: 'input_value', name, check: 'RuleCondition' }
}

function textInputShadows(values: Record<string, string>) {
  return Object.fromEntries(
    Object.entries(values).map(([name, value]) => [name, textInputShadow(value)])
  )
}

function textInputShadow(value: string) {
  return {
    shadow: {
      type: TEXT_LITERAL_BLOCK_TYPE,
      fields: { TEXT: value },
    },
  }
}

function selectorInputShadow(kind: BlockKindSelection) {
  return {
    shadow: {
      type: SELECTOR_BLOCK_TYPE,
      fields: { KIND: kind, TEXT_MODE: 'none', INDEX_MODE: 'all', N: '0', END: '0' },
    },
  }
}

function positionInputShadow(pos: PositionSelection) {
  return {
    shadow: {
      type: POSITION_BLOCK_TYPE,
      fields: { POS: pos, N: '0' },
      inputs: {
        SELECTOR: selectorInputShadow('paragraph'),
      },
    },
  }
}

function conditionInputShadow() {
  return {
    shadow: {
      type: CONDITION_COUNT_BLOCK_TYPE,
      fields: { OP: 'equals', N: '1' },
      inputs: {
        SELECTOR: selectorInputShadow('paragraph'),
      },
    },
  }
}

function setBlocklyTextInput(block: Blockly.Block, inputName: string, value: string) {
  const connection = block.getInput(inputName)?.connection
  if (!connection) {
    setBlocklyField(block, inputName, value)
    return
  }

  connection.targetBlock()?.dispose(false)
  const textBlock = block.workspace.newBlock(TEXT_LITERAL_BLOCK_TYPE) as Blockly.BlockSvg
  textBlock.setShadow(true)
  setBlocklyField(textBlock, 'TEXT', value)
  textBlock.initSvg()
  textBlock.render()
  if (textBlock.outputConnection) {
    connection.connect(textBlock.outputConnection)
  }
}

function setBlocklySelectorInput(block: Blockly.Block, inputName: string, selector: BlockSelector) {
  setBlocklyOutputInput(block, inputName, createSelectorBlock(block.workspace, selector))
}

function setBlocklyPositionInput(block: Blockly.Block, inputName: string, position: PositionSpec) {
  setBlocklyOutputInput(block, inputName, createPositionBlock(block.workspace, position))
}

function setBlocklyConditionInput(block: Blockly.Block, inputName: string, condition: RuleCondition) {
  setBlocklyOutputInput(block, inputName, createConditionBlock(block.workspace, condition))
}

function setBlocklyOutputInput(block: Blockly.Block, inputName: string, valueBlock: Blockly.Block) {
  const connection = block.getInput(inputName)?.connection
  if (!connection) {
    valueBlock.dispose(false)
    return
  }

  connection.targetBlock()?.dispose(false)
  if (valueBlock.outputConnection) {
    connection.connect(valueBlock.outputConnection)
  }
}

function setBlocklyField(block: Blockly.Block, fieldName: string, value: string) {
  if (block.getField(fieldName)) {
    block.setFieldValue(value, fieldName)
  }
}

function readBlocklyField(block: Blockly.Block, fieldName: string) {
  return block.getFieldValue(fieldName) ?? ''
}

function readBlocklyTextInput(block: Blockly.Block, inputName: string) {
  const targetBlock = block.getInputTargetBlock(inputName)
  return targetBlock ? readBlocklyTextBlock(targetBlock) : readBlocklyField(block, inputName)
}

function readBlocklyTextBlock(block: Blockly.Block): string {
  switch (block.type) {
    case TEXT_LITERAL_BLOCK_TYPE:
      return readBlocklyField(block, 'TEXT')
    case VARIABLE_TOKEN_BLOCK_TYPE:
      return readBlocklyField(block, 'TOKEN')
    case JOIN_TEXT_BLOCK_TYPE:
      return `${readBlocklyTextInput(block, 'LEFT')}${readBlocklyTextInput(block, 'RIGHT')}`
    default:
      return ''
  }
}

function joinListField(values: string[]) {
  return values.join(', ')
}

function parseListField(value: string) {
  return value
    .split(/[\n,，]/)
    .map((item) => item.trim())
    .filter(Boolean)
}
