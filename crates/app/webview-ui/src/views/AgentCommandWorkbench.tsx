import { useEffect, useMemo, useRef, useState } from 'react'
import { Button, Input } from '@heroui/react'
import * as Blockly from 'scratch-blocks'
import * as ZhHans from 'blockly/msg/zh-hans'
import {
  ArrowDown,
  ArrowUp,
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
  AgentCommandQueueInsertPosition,
  AgentCommandReviewAction,
  AgentCommandShortcutScope,
  AppConfigAgentCommand,
  AppConfigSettingsResponse,
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
const VARIABLE_TOKEN_BLOCK_TYPE = 'oqqwall_variable_token'
const TEXT_LITERAL_BLOCK_TYPE = 'oqqwall_text_literal'
const JOIN_TEXT_BLOCK_TYPE = 'oqqwall_join_text'
const SCRATCH_BLOCKS_MEDIA_PATH = '/scratch-blocks-media/'

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
      colourPrimary: '#171717',
      colourSecondary: '#0a0a0a',
      colourTertiary: '#0a0a0a',
    },
    message_blocks: {
      colourPrimary: '#404040',
      colourSecondary: '#262626',
      colourTertiary: '#171717',
    },
    session_blocks: {
      colourPrimary: '#525252',
      colourSecondary: '#404040',
      colourTertiary: '#262626',
    },
    review_blocks: {
      colourPrimary: '#737373',
      colourSecondary: '#525252',
      colourTertiary: '#404040',
    },
    system_blocks: {
      colourPrimary: '#262626',
      colourSecondary: '#171717',
      colourTertiary: '#0a0a0a',
    },
    variable_blocks: {
      colourPrimary: '#5f5f5f',
      colourSecondary: '#404040',
      colourTertiary: '#262626',
    },
  },
  categoryStyles: {
    event_category: { colour: '#171717' },
    message_category: { colour: '#404040' },
    session_category: { colour: '#525252' },
    review_category: { colour: '#737373' },
    system_category: { colour: '#262626' },
    variable_category: { colour: '#5f5f5f' },
  },
  componentStyles: {
    workspaceBackgroundColour: '#f5f5f5',
    toolboxBackgroundColour: '#fafafa',
    toolboxForegroundColour: '#0a0a0a',
    flyoutBackgroundColour: '#ffffff',
    flyoutForegroundColour: '#0a0a0a',
    flyoutOpacity: 1,
    scrollbarColour: '#737373',
    insertionMarkerColour: '#171717',
    selectedGlowColour: '#171717',
    selectedGlowOpacity: 0.22,
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

  function moveCommand(commandIndex: number, direction: -1 | 1) {
    const nextCommands = moveArrayItem(commands, commandIndex, direction)
    updateCommands(nextCommands)
    setSelectedCommandIndex(commandIndex + direction)
  }

  const selectedCommandKey = selectedCommand
    ? `command-${selectedCommandIndex}-${commands.length}`
    : 'empty'

  return (
    <div className="settings-panel scratch-workbench">
      <div className="settings-section-head">
        <div>
          <span className="field-label">Scratch Agent 指令</span>
          <p className="field-hint">
            每个触发词对应一个 Blockly 工作区，保存时仍写回原来的 agent_commands 结构。
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
              {commands.map((command, commandIndex) => (
                <button
                  key={`${command.name}-${commandIndex}`}
                  type="button"
                  className={`scratch-command-tab${
                    commandIndex === selectedCommandIndex ? ' is-active' : ''
                  }${command.enabled ? '' : ' is-disabled'}`}
                  onClick={() => setSelectedCommandIndex(commandIndex)}
                >
                  <strong>#{command.name || `command_${commandIndex + 1}`}</strong>
                  <span>{command.description || (command.enabled ? '已启用' : '已关闭')}</span>
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
                <div className="scratch-command-actions">
                  <Button
                    size="sm"
                    variant="secondary"
                    isIconOnly
                    aria-label="上移指令"
                    isDisabled={selectedCommandIndex === 0}
                    onClick={() => moveCommand(selectedCommandIndex, -1)}
                  >
                    <ArrowUp size={16} />
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    isIconOnly
                    aria-label="下移指令"
                    isDisabled={selectedCommandIndex >= commands.length - 1}
                    onClick={() => moveCommand(selectedCommandIndex, 1)}
                  >
                    <ArrowDown size={16} />
                  </Button>
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
                variables={variables}
                onChange={(blocks) =>
                  updateCommand(selectedCommandIndex, (command) => ({ ...command, blocks }))
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
  variables,
  onChange,
}: {
  blocks: AgentCommandBlock[]
  variables: AgentCommandVariable[]
  onChange: (blocks: AgentCommandBlock[]) => void
}) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const workspaceRef = useRef<Blockly.WorkspaceSvg | null>(null)
  const onChangeRef = useRef(onChange)
  const initialBlocksRef = useRef(blocks)
  const [isFullscreen, setFullscreen] = useState(false)
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
        colour: '#e5e5e5',
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

    loadAgentBlocksIntoWorkspace(workspace, initialBlocksRef.current)

    const listener = (event: Blockly.Events.Abstract) => {
      if (event.isUiEvent) return
      onChangeRef.current(workspaceToAgentBlocks(workspace))
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
      message0: '当 Agent 指令被触发',
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
    },
    {
      type: 'oqqwall_finish_submission_session',
      message0: '结束投稿会话并等待确认',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
    },
    {
      type: 'oqqwall_resume_submission_session',
      message0: '继续编辑投稿会话',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
    },
    {
      type: 'oqqwall_submit_submission_session',
      message0: '提交投稿会话',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
    },
    {
      type: 'oqqwall_cancel_submission_session',
      message0: '取消投稿会话',
      previousStatement: null,
      nextStatement: null,
      style: 'session_blocks',
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
  blocks: AgentCommandBlock[]
) {
  Blockly.Events.disable()
  try {
    workspace.clear()
    const root = workspace.newBlock(ROOT_BLOCK_TYPE)
    root.setDeletable(false)
    root.setMovable(false)
    root.initSvg()
    root.render()
    root.moveBy(28, 28)

    let previousBlock: Blockly.Block | null = root
    blocks.forEach((agentBlock) => {
      const block = createBlocklyBlock(workspace, agentBlock)
      if (!block || !previousBlock?.nextConnection || !block.previousConnection) return
      previousBlock.nextConnection.connect(block.previousConnection)
      previousBlock = block
    })

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
    default:
      break
  }

  block.render()
  return block
}

function workspaceToAgentBlocks(workspace: Blockly.WorkspaceSvg): AgentCommandBlock[] {
  const topBlocks = workspace.getTopBlocks(true)
  const root = topBlocks.find((block) => block.type === ROOT_BLOCK_TYPE)
  const blocks: AgentCommandBlock[] = []

  if (root) {
    appendBlocklyStack(blocks, root.getNextBlock())
  }

  topBlocks
    .filter((block) => block.type !== ROOT_BLOCK_TYPE && isAgentStatementBlock(block))
    .forEach((block) => appendBlocklyStack(blocks, block))

  return blocks
}

function appendBlocklyStack(out: AgentCommandBlock[], firstBlock: Blockly.Block | null) {
  let currentBlock = firstBlock
  while (currentBlock) {
    const agentBlock = blocklyBlockToAgentBlock(currentBlock)
    if (agentBlock) out.push(agentBlock)
    currentBlock = currentBlock.getNextBlock()
  }
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

function moveArrayItem<T>(items: T[], index: number, direction: -1 | 1) {
  const targetIndex = index + direction
  if (targetIndex < 0 || targetIndex >= items.length) return items
  const next = [...items]
  const [item] = next.splice(index, 1)
  next.splice(targetIndex, 0, item)
  return next
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

function dropdownOptions<T extends string>(options: Array<{ value: T; label: string }>) {
  return options.map((option) => [option.label, option.value])
}

function textValueArg(name: string) {
  return { type: 'input_value', name, check: 'String' }
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
