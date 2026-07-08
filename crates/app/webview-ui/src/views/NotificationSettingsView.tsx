import {
  type CSSProperties,
  type DragEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  Button,
  Card,
  Checkbox,
  Chip,
  Drawer,
  Input,
  Pagination,
  Popover,
  Spinner,
  Switch,
  TextArea,
  ToggleButton,
  ToggleButtonGroup,
} from '@heroui/react'
import {
  AlertCircle,
  BarChart3,
  Ban,
  Check,
  CheckCircle2,
  Clock3,
  FileText,
  HelpCircle,
  Inbox,
  LayoutGrid,
  List,
  MessageSquare,
  MoreHorizontal,
  PanelRightOpen,
  RefreshCcw,
  Search,
  Send,
  ShieldCheck,
  Trash2,
  UserRound,
  X,
  Zap,
} from 'lucide-react'
import { api } from '../api/client'
import {
  ACTION_LABELS,
  AgentCommandBlock,
  AgentCommandGlobalAction,
  AgentCommandQueueInsertPosition,
  AgentCommandReviewAction,
  AgentCommandShortcutScope,
  AppConfigAgentCommand,
  AppConfigCommonSettings,
  AppConfigGroupSettings,
  AppConfigSettingsResponse,
  ListPostsResponse,
  ListReviewIdsResponse,
  MappingEntry,
  MeResponse,
  PostDetail,
  PostItem,
  Stage,
  StatsResponse,
  ConfigAdminEntry,
  TagValueMappingEntry,
  TagValueMappingGroup,
  UserNotificationSettingsResponse,
  UserNotificationTemplate,
} from '../api/types'
import {
  ACTIVE_EXCLUDED,
  AGENT_BLOCK_DRAG_MIME,
  AGENT_COMMAND_BLOCK_OPTIONS,
  AGENT_GLOBAL_ACTION_OPTIONS,
  AGENT_QUEUE_INSERT_POSITION_OPTIONS,
  AGENT_REVIEW_ACTION_OPTIONS,
  AGENT_SHORTCUT_SCOPE_OPTIONS,
  BATCH_ACTIONS,
  CARD_QUICK_ACTIONS,
  DANGEROUS_ACTIONS,
  DETAIL_ACTIONS,
  DETAIL_QUICK_ACTIONS,
  LIST_PRIMARY_ACTIONS,
  NOTIFICATION_STAGE_OPTIONS,
  PAGE_SIZES,
  SORT_OPTIONS,
  type AgentBlockDragPayload,
  type AgentCommandBlockOptionValue,
  type AgentCommandFieldNode,
  type AgentGlobalActionKey,
  type AgentReviewActionKey,
  type AgentVariableDragPayload,
  type ConfigMenuKey,
  type ExecuteGlobalActionBlock,
  type ExecuteReviewActionBlock,
  type InsertQueuedPostBlock,
  type NotificationMenuKey,
  type NotificationStageKey,
  type PostQuerySnapshot,
  type PostViewMode,
  type ReplyPrivateMessageBlock,
  type RuntimeConfigPageKey,
  type RuntimeConfigWorkbenchMode,
  type SelectOption,
  type SendWebhookBlock,
  type SettingsTabKey,
  type SortOrder,
  type TagMappingMenuKey,
  type ToastKind,
  type PostsWorkspaceMode,
  buildActionPayload,
  buildPostParams,
  buildReplyImageLabel,
  cardActionLabel,
  DelayField,
  EmptyPanel,
  formatDateTime,
  formatDuration,
  HeroSelect,
  isPreviewableImageSource,
  MappingListEditor,
  Metric,
  quickActionIcon,
  quickActionVariant,
  readFileAsDataUrl,
  SettingsMenuCard,
  StageChip,
  useMasonryLayout,
  useMediaQuery,
} from '../shared'

function NotificationStageCardPreview({ images }: { images: string[] }) {
  const previewable = images
    .map((value) => value.trim())
    .filter((value) => isPreviewableImageSource(value))
    .slice(0, 3)

  if (!previewable.length) {
    return (
      <div className="stage-card-media stage-card-media-empty">
        <span>未附图</span>
      </div>
    )
  }

  return (
    <div className="stage-card-media">
      {previewable.map((image, index) => (
        <span key={`${image}-${index}`} className="stage-card-media-item">
          <img src={image} alt={`阶段图片预览 ${index + 1}`} loading="lazy" />
        </span>
      ))}
      {images.length > previewable.length ? (
        <span className="stage-card-media-more">+{images.length - previewable.length}</span>
      ) : null}
    </div>
  )
}

function StageTagManager({
  title,
  tags,
  onAdd,
  onChange,
  onRemove,
}: {
  title: string
  tags: string[]
  onAdd: () => void
  onChange: (index: number, value: string) => void
  onRemove: (index: number) => void
}) {
  return (
    <Card className="panel-card">
      <Card.Header>
        <Card.Title>{title}</Card.Title>
      </Card.Header>
      <Card.Content>
        <div className="settings-panel">
          <div className="settings-section-head">
            <span className="field-label">标签列表</span>
            <Button size="sm" variant="secondary" onClick={onAdd}>
              新增标签
            </Button>
          </div>
          {tags.length ? (
            <div className="settings-variable-list">
              {tags.map((tag, index) => (
                <div key={`${title}-${index}`} className="settings-variable-card">
                  <div className="settings-image-row">
                    <Input
                      className="settings-image-input"
                      placeholder="例如 <source_webhook_tag> 或 3344846508"
                      value={tag}
                      onChange={(event) => onChange(index, event.target.value)}
                    />
                    <Button
                      size="sm"
                      variant="tertiary"
                      isIconOnly
                      aria-label={`删除标签 ${index + 1}`}
                      onClick={() => onRemove(index)}
                    >
                      <Trash2 size={16} />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className="settings-empty">
              <MessageSquare size={20} />
              <span>当前阶段还没有配置标签。</span>
            </div>
          )}
        </div>
      </Card.Content>
    </Card>
  )
}

function UserNotificationSettingsView({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  /*
  const [settings, setSettings] = useState<UserNotificationSettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [selectedStage, setSelectedStage] = useState<NotificationStageKey>('send_succeeded')
  const [selectedMenu, setSelectedMenu] = useState<NotificationMenuKey>('stages')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const uploadInputRef = useRef<HTMLInputElement | null>(null)
  const templateTextareaRef = useRef<HTMLTextAreaElement | null>(null)

  useEffect(() => {
    void loadSettings()
  }, [])

  function applySettings(result: UserNotificationSettingsResponse) {
    setSettings(result)
    setSelectedGroup(result.group_id)
  }

  async function loadSettings(groupId?: string) {
    setLoading(true)
    try {
      const params = new URLSearchParams()
      const targetGroup = (groupId ?? selectedGroup).trim()
      if (targetGroup) params.set('group_id', targetGroup)
      const suffix = params.toString() ? `?${params.toString()}` : ''
      const result = await api<UserNotificationSettingsResponse>(
        `/api/settings/user-notifications${suffix}`
      )
      applySettings(result)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!settings || !selectedGroup) {
      notify('error', '请先选择要配置的群组')
      return
    }
    setSaving(true)
    try {
      const result = await api<UserNotificationSettingsResponse>(
        '/api/settings/user-notifications',
        {
          method: 'POST',
          body: JSON.stringify({
            group_id: selectedGroup,
            queue_entered: settings.queue_entered,
            review_queued: settings.review_queued,
            send_succeeded: settings.send_succeeded,
            rejected: settings.rejected,
            webhook_tag_map: settings.webhook_tag_map,
            tag_value_maps: settings.tag_value_maps,
          }),
        }
      )
      applySettings(result)
      notify('success', '回链设置已保存，并同步到运行时')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function updateStageTemplate(
    updater: (template: UserNotificationTemplate) => UserNotificationTemplate
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [selectedStage]: updater(prev[selectedStage]),
      }
    })
  }

  function insertVariable(key: string) {
    const token = `<${key}>`
    const textarea = templateTextareaRef.current
    const currentText = activeTemplate.text_template
    const selectionStart = textarea?.selectionStart ?? currentText.length
    const selectionEnd = textarea?.selectionEnd ?? currentText.length
    const nextText = `${currentText.slice(0, selectionStart)}${token}${currentText.slice(selectionEnd)}`
    updateStageTemplate((template) => ({
      ...template,
      text_template: nextText,
    }))
    window.requestAnimationFrame(() => {
      const nextCaret = selectionStart + token.length
      templateTextareaRef.current?.focus()
      templateTextareaRef.current?.setSelectionRange(nextCaret, nextCaret)
    })
  }

  function updateStageList(field: 'tags' | 'images', index: number, value: string) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].map((item, itemIndex) => (itemIndex === index ? value : item)),
    }))
  }

  function addStageListItem(field: 'tags' | 'images') {
    updateStageTemplate((template) => ({
      ...template,
      [field]: [...template[field], ''],
    }))
  }

  function removeStageListItem(field: 'tags' | 'images', index: number) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateMapping(
    field: 'webhook_tag_map',
    index: number,
    key: keyof MappingEntry,
    value: string
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].map((item, itemIndex) =>
          itemIndex === index ? { ...item, [key]: value } : item
        ),
      }
    })
  }

  function addMapping(field: 'webhook_tag_map') {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: [...prev[field], { key: '', value: '' }],
      }
    })
  }

  function removeMapping(field: 'webhook_tag_map', index: number) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].filter((_, itemIndex) => itemIndex !== index),
      }
    })
  }

  async function uploadImages(event: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? [])
    if (!files.length) return
    try {
      const uploaded = await Promise.all(files.map(readFileAsDataUrl))
      updateStageTemplate((template) => ({
        ...template,
        images: [...template.images, ...uploaded],
      }))
      notify('success', `已添加 ${uploaded.length} 张图片`)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      event.target.value = ''
    }
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载用户回链设置" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>用户回链</h1>
            <p>配置进入队列、发稿成功、拒稿三种阶段的用户通知。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<Send size={28} />} text="当前没有可用的回链设置数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const activeStage = NOTIFICATION_STAGE_OPTIONS.find((item) => item.value === selectedStage)!
  const activeTemplate = settings[selectedStage]
  const groupOptions = settings.available_groups.map((group) => ({ value: group, label: group }))

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>用户回链</h1>
          <p>支持 webhook 映射标签、标签值重写，以及三阶段自定义文案、标签和图片。</p>
        </div>
        <div className="head-actions">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading || saving}
            onClick={() => void loadSettings(selectedGroup)}
          >
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="settings-grid">
        <Card className="panel-card settings-main-card">
          <Card.Header>
            <Card.Title>阶段模板</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-toolbar">
                <div className="field-stack">
                  <span className="field-label">群组</span>
                  <HeroSelect
                    className="control settings-group-select"
                    ariaLabel="选择群组"
                    selectedKey={selectedGroup}
                    options={groupOptions}
                    onSelect={(value) => {
                      setSelectedGroup(value)
                      void loadSettings(value)
                    }}
                  />
                </div>
              </div>

              <div className="settings-stage-grid">
                {NOTIFICATION_STAGE_OPTIONS.map((stage) => (
                  <button
                    key={stage.value}
                    type="button"
                    className={`settings-stage-button ${
                      selectedStage === stage.value ? 'is-active' : ''
                    }`}
                    onClick={() => setSelectedStage(stage.value)}
                  >
                    <span className="settings-stage-icon">{stage.icon}</span>
                    <strong>{stage.label}</strong>
                    <span>{stage.description}</span>
                  </button>
                ))}
              </div>

              <div className="settings-inline-grid">
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">当前阶段</span>
                  <span className="field-hint">{activeStage.label}</span>
                </div>
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">启用通知</span>
                  <Switch
                    isSelected={activeTemplate.enabled}
                    onChange={(value) =>
                      updateStageTemplate((template) => ({ ...template, enabled: value }))
                    }
                    size="sm"
                  >
                    {activeTemplate.enabled ? '已启用' : '已关闭'}
                  </Switch>
                </div>
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">自动带上稿件标签</span>
                  <Switch
                    isSelected={activeTemplate.include_post_tags}
                    onChange={(value) =>
                      updateStageTemplate((template) => ({
                        ...template,
                        include_post_tags: value,
                      }))
                    }
                    size="sm"
                  >
                    {activeTemplate.include_post_tags ? '已启用' : '已关闭'}
                  </Switch>
                </div>
              </div>

              <div className="field-stack">
                <span className="field-label">通知文案</span>
                <TextArea
                  ref={templateTextareaRef}
                  className="settings-template-textarea"
                  placeholder="#<code> 已发稿"
                  value={activeTemplate.text_template}
                  onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                    updateStageTemplate((template) => ({
                      ...template,
                      text_template: event.target.value,
                    }))
                  }
                />
                <span className="field-hint">
                  支持使用 <code>{'<变量>'}</code> 占位。标签会显示在正文上方，图片按顺序附加。
                </span>
              </div>

              <div className="field-stack">
                <div className="settings-section-head">
                  <span className="field-label">可插入变量</span>
                  <span className="field-hint">点击后会在当前光标处插入变量，标签和图片地址同样支持变量。</span>
                </div>
                <div className="settings-variable-grid">
                  {settings.variables.map((variable) => (
                    <button
                      key={variable.key}
                      type="button"
                      className="settings-variable-button"
                      onClick={() => insertVariable(variable.key)}
                    >
                      <strong>{variable.label}</strong>
                      <code>{variable.example}</code>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>标签与图片</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-subsection">
                <div className="settings-section-head">
                  <span className="field-label">额外标签</span>
                  <Button size="sm" variant="secondary" onClick={() => addStageListItem('tags')}>
                    新增标签
                  </Button>
                </div>
                <span className="field-hint">
                  每个标签都支持变量，占位后会经过标签值映射表再发送。
                </span>
                {activeTemplate.tags.length ? (
                  <div className="settings-image-list">
                    {activeTemplate.tags.map((tag, index) => (
                      <div key={`tag-${selectedStage}-${index}`} className="settings-image-item">
                        <div className="settings-image-row">
                          <Input
                            className="settings-image-input"
                            placeholder="例如 <source_webhook_tag> 或 3344846508"
                            value={tag}
                            onChange={(event) => updateStageList('tags', index, event.target.value)}
                          />
                          <Button
                            size="sm"
                            variant="tertiary"
                            isIconOnly
                            aria-label={`删除标签 ${index + 1}`}
                            onClick={() => removeStageListItem('tags', index)}
                          >
                            <Trash2 size={16} />
              </Button>
            </div>
          </div>
        ))}
      </div>
                ) : (
                  <div className="settings-empty">
                    <MessageSquare size={20} />
                    <span>当前阶段未配置额外标签。</span>
                  </div>
                )}
              </div>

              <div className="settings-subsection">
                <div className="settings-image-actions">
                  <Button size="sm" variant="secondary" onClick={() => addStageListItem('images')}>
                    新增图片
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => uploadInputRef.current?.click()}
                  >
                    上传图片
                  </Button>
                  <input
                    ref={uploadInputRef}
                    className="sr-only"
                    type="file"
                    accept="image/*"
                    multiple
                    onChange={uploadImages}
                  />
                </div>
                <span className="field-hint">
                  可填写远程图片 URL，也可直接上传，上传后会保存为 data URL。
                </span>
                {activeTemplate.images.length ? (
                  <div className="settings-image-list">
                    {activeTemplate.images.map((image, index) => {
                      const previewable = isPreviewableImageSource(image)
                      return (
                        <div
                          key={`reply-image-${selectedStage}-${index}`}
                          className="settings-image-item"
                        >
                          <div className="settings-image-row">
                            <Input
                              className="settings-image-input"
                              placeholder="https://example.com/success.png 或 data:image/..."
                              value={image}
                              onChange={(event) =>
                                updateStageList('images', index, event.target.value)
                              }
                            />
                            <Button
                              size="sm"
                              variant="tertiary"
                              isIconOnly
                              aria-label={`删除图片 ${index + 1}`}
                              onClick={() => removeStageListItem('images', index)}
                            >
                              <Trash2 size={16} />
                            </Button>
                          </div>
                          <div className="settings-image-meta">
                            <span className="field-hint">{buildReplyImageLabel(image, index)}</span>
                          </div>
                          {previewable && (
                            <div className="settings-image-preview-frame">
                              <img
                                className="settings-image-preview"
                                src={image.trim()}
                                alt={`回链图片预览 ${index + 1}`}
                              />
                            </div>
                          )}
                        </div>
                      )
                    })}
                  </div>
                ) : (
                  <div className="settings-empty">
                    <MessageSquare size={20} />
                    <span>当前阶段未附带图片，保存后只发送标签和文案。</span>
                  </div>
                )}
              </div>
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>Webhook 到标签</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-section-head">
                <span className="field-hint">将 webhook 标识映射为默认标签。</span>
                <Button size="sm" variant="secondary" onClick={() => addMapping('webhook_tag_map')}>
                  新增映射
                </Button>
              </div>
              {settings.webhook_tag_map.length ? (
                <div className="settings-variable-list">
                  {settings.webhook_tag_map.map((entry, index) => (
                    <div key={`webhook-map-${index}`} className="settings-variable-card">
                      <div className="settings-image-row">
                        <Input
                          className="settings-image-input"
                          placeholder="Webhook"
                          value={entry.key}
                          onChange={(event) =>
                            updateMapping('webhook_tag_map', index, 'key', event.target.value)
                          }
                        />
                        <Input
                          className="settings-image-input"
                          placeholder="标签"
                          value={entry.value}
                          onChange={(event) =>
                            updateMapping('webhook_tag_map', index, 'value', event.target.value)
                          }
                        />
                        <Button
                          size="sm"
                          variant="tertiary"
                          isIconOnly
                          aria-label={`删除 webhook 映射 ${index + 1}`}
                          onClick={() => removeMapping('webhook_tag_map', index)}
                        >
                          <Trash2 size={16} />
                        </Button>
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="settings-empty">
                  <MessageSquare size={20} />
                  <span>当前没有 webhook 映射。</span>
                </div>
              )}
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>变量说明</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-variable-list">
              {settings.variables.map((variable) => (
                <div key={variable.key} className="settings-variable-card">
                  <div className="settings-variable-meta">
                    <strong>{variable.label}</strong>
                    <code>{variable.example}</code>
                  </div>
                  <p>{variable.description}</p>
                </div>
              ))}
            </div>
          </Card.Content>
        </Card>
      </section>
    </div>
  )
  */
  return <NotificationSettingsWorkbench notify={notify} />
}

export function NotificationSettingsWorkbench({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  const [settings, setSettings] = useState<UserNotificationSettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [selectedStage, setSelectedStage] = useState<NotificationStageKey>('send_succeeded')
  const [selectedMenu, setSelectedMenu] = useState<NotificationMenuKey>('stages')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const uploadInputRef = useRef<HTMLInputElement | null>(null)
  const templateTextareaRef = useRef<HTMLTextAreaElement | null>(null)

  useEffect(() => {
    void loadSettings()
  }, [])

  async function loadSettings(groupId?: string) {
    setLoading(true)
    try {
      const params = new URLSearchParams()
      const targetGroup = (groupId ?? selectedGroup).trim()
      if (targetGroup) params.set('group_id', targetGroup)
      const suffix = params.toString() ? `?${params.toString()}` : ''
      const result = await api<UserNotificationSettingsResponse>(
        `/api/settings/user-notifications${suffix}`
      )
      setSettings(result)
      setSelectedGroup(result.group_id)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!settings || !selectedGroup) {
      notify('error', '请先选择要配置的群组')
      return
    }
    setSaving(true)
    try {
      const result = await api<UserNotificationSettingsResponse>(
        '/api/settings/user-notifications',
        {
          method: 'POST',
          body: JSON.stringify({
            group_id: selectedGroup,
            queue_entered: settings.queue_entered,
            review_queued: settings.review_queued,
            send_succeeded: settings.send_succeeded,
            rejected: settings.rejected,
            webhook_tag_map: settings.webhook_tag_map,
          }),
        }
      )
      setSettings(result)
      setSelectedGroup(result.group_id)
      notify('success', '用户回链设置已保存')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function updateStageTemplate(
    updater: (template: UserNotificationTemplate) => UserNotificationTemplate
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [selectedStage]: updater(prev[selectedStage]),
      }
    })
  }

  function updateStageList(field: 'tags' | 'images', index: number, value: string) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].map((item, itemIndex) => (itemIndex === index ? value : item)),
    }))
  }

  function addStageListItem(field: 'tags' | 'images') {
    updateStageTemplate((template) => ({
      ...template,
      [field]: [...template[field], ''],
    }))
  }

  function removeStageListItem(field: 'tags' | 'images', index: number) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateMapping(
    field: 'webhook_tag_map',
    index: number,
    key: keyof MappingEntry,
    value: string
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].map((item, itemIndex) =>
          itemIndex === index ? { ...item, [key]: value } : item
        ),
      }
    })
  }

  function addMapping(field: 'webhook_tag_map') {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: [...prev[field], { key: '', value: '' }],
      }
    })
  }

  function removeMapping(field: 'webhook_tag_map', index: number) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].filter((_, itemIndex) => itemIndex !== index),
      }
    })
  }

  async function uploadImages(event: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? [])
    if (!files.length) return
    try {
      const uploaded = await Promise.all(files.map(readFileAsDataUrl))
      updateStageTemplate((template) => ({
        ...template,
        images: [...template.images, ...uploaded],
      }))
      notify('success', `已添加 ${uploaded.length} 张图片`)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      event.target.value = ''
    }
  }

  function insertVariable(key: string) {
    if (!settings) return
    const currentText = settings[selectedStage].text_template
    const textarea = templateTextareaRef.current
    const token = `<${key}>`
    const selectionStart = textarea?.selectionStart ?? currentText.length
    const selectionEnd = textarea?.selectionEnd ?? currentText.length
    updateStageTemplate((template) => ({
      ...template,
      text_template: `${currentText.slice(0, selectionStart)}${token}${currentText.slice(selectionEnd)}`,
    }))
    window.requestAnimationFrame(() => {
      const nextCaret = selectionStart + token.length
      templateTextareaRef.current?.focus()
      templateTextareaRef.current?.setSelectionRange(nextCaret, nextCaret)
    })
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载用户回链设置" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>用户回链</h1>
            <p>配置进入队列、发稿成功、拒稿三个阶段的用户通知。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<Send size={28} />} text="当前没有可用的回链设置数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const activeStage = NOTIFICATION_STAGE_OPTIONS.find((item) => item.value === selectedStage)!
  const activeTemplate = settings[selectedStage]
  const groupOptions = settings.available_groups.map((group) => ({ value: group, label: group }))
  const menuOptions = [
    {
      value: 'stages' as const,
      label: '阶段模板',
      icon: <Send size={16} />,
    },
    {
      value: 'webhooks' as const,
      label: 'Webhook 标签',
      icon: <Zap size={16} />,
    },
    {
      value: 'variables' as const,
      label: '变量说明',
      icon: <HelpCircle size={16} />,
    },
  ]

  function renderGroupCard(extra?: ReactNode) {
    return (
      <Card className="panel-card">
        <Card.Content>
          <div className="settings-toolbar">
            <div className="field-stack">
              <span className="field-label">群组</span>
              <HeroSelect
                className="control settings-group-select"
                ariaLabel="选择群组"
                selectedKey={selectedGroup}
                options={groupOptions}
                onSelect={(value) => {
                  setSelectedGroup(value)
                  void loadSettings(value)
                }}
              />
            </div>
            {extra}
          </div>
        </Card.Content>
      </Card>
    )
  }

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>用户回链</h1>
          <p>将阶段模板、Webhook 标签和变量说明拆成独立菜单，方便单独维护。</p>
        </div>
        <div className="head-actions">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading || saving}
            onClick={() => void loadSettings(selectedGroup)}
          >
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="settings-layout">
        <SettingsMenuCard
          title="回链菜单"
          options={menuOptions}
          activeKey={selectedMenu}
          onSelect={setSelectedMenu}
        />

        <div className="settings-content-stack">
          {selectedMenu === 'stages' ? (
            <>
              {renderGroupCard()}

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>阶段选择</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="settings-stage-grid stage-selector-grid">
                    {NOTIFICATION_STAGE_OPTIONS.map((stage) => {
                      const template = settings[stage.value]
                      return (
                        <button
                          key={stage.value}
                          type="button"
                          className={`settings-stage-button settings-stage-card ${
                            selectedStage === stage.value ? 'is-active' : ''
                          }`}
                          onClick={() => setSelectedStage(stage.value)}
                        >
                          <span className="settings-stage-icon">{stage.icon}</span>
                          <strong>{stage.label}</strong>
                          <div className="stage-card-metrics">
                            <span>{template.tags.length} 个标签</span>
                            <span>{template.images.length} 张图片</span>
                          </div>
                          <NotificationStageCardPreview images={template.images} />
                        </button>
                      )
                    })}
                  </div>
                </Card.Content>
              </Card>

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>{activeStage.label}</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="settings-panel">
                    <div className="settings-inline-grid">
                      <div className="field-stack settings-switch-stack">
                        <span className="field-label">当前阶段</span>
                        <span className="field-hint">{activeStage.label}</span>
                      </div>
                      <div className="field-stack settings-switch-stack">
                        <span className="field-label">启用通知</span>
                        <Switch
                          isSelected={activeTemplate.enabled}
                          onChange={(value) =>
                            updateStageTemplate((template) => ({ ...template, enabled: value }))
                          }
                          size="sm"
                        >
                          {activeTemplate.enabled ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                      <div className="field-stack settings-switch-stack">
                        <span className="field-label">自动带上稿件标签</span>
                        <Switch
                          isSelected={activeTemplate.include_post_tags}
                          onChange={(value) =>
                            updateStageTemplate((template) => ({
                              ...template,
                              include_post_tags: value,
                            }))
                          }
                          size="sm"
                        >
                          {activeTemplate.include_post_tags ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                    </div>

                    <div className="field-stack">
                      <span className="field-label">通知文案</span>
                      <TextArea
                        ref={templateTextareaRef}
                        className="settings-template-textarea"
                        placeholder="#<code> 已处理"
                        value={activeTemplate.text_template}
                        onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                          updateStageTemplate((template) => ({
                            ...template,
                            text_template: event.target.value,
                          }))
                        }
                      />
                    </div>

                    <div className="field-stack">
                      <div className="settings-section-head">
                        <span className="field-label">可插入变量</span>
                        <span className="field-hint">点击后会在当前光标处插入变量。</span>
                      </div>
                      <div className="settings-variable-grid">
                        {settings.variables.map((variable) => (
                          <button
                            key={variable.key}
                            type="button"
                            className="settings-variable-button"
                            onClick={() => insertVariable(variable.key)}
                          >
                            <strong>{variable.label}</strong>
                            <code>{variable.example}</code>
                          </button>
                        ))}
                      </div>
                    </div>

                    <div className="settings-subsection">
                      <div className="settings-section-head">
                        <span className="field-label">阶段图片</span>
                        <div className="settings-image-actions">
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => addStageListItem('images')}
                          >
                            新增图片
                          </Button>
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => uploadInputRef.current?.click()}
                          >
                            上传图片
                          </Button>
                          <input
                            ref={uploadInputRef}
                            className="sr-only"
                            type="file"
                            accept="image/*"
                            multiple
                            onChange={uploadImages}
                          />
                        </div>
                      </div>
                      {activeTemplate.images.length ? (
                        <div className="settings-image-list">
                          {activeTemplate.images.map((image: string, index: number) => {
                            const previewable = isPreviewableImageSource(image)
                            return (
                              <div
                                key={`stage-image-${selectedStage}-${index}`}
                                className="settings-image-item"
                              >
                                <div className="settings-image-row">
                                  <Input
                                    className="settings-image-input"
                                    placeholder="https://example.com/success.png 或 data:image/..."
                                    value={image}
                                    onChange={(event) =>
                                      updateStageList('images', index, event.target.value)
                                    }
                                  />
                                  <Button
                                    size="sm"
                                    variant="tertiary"
                                    isIconOnly
                                    aria-label={`删除图片 ${index + 1}`}
                                    onClick={() => removeStageListItem('images', index)}
                                  >
                                    <Trash2 size={16} />
                                  </Button>
                                </div>
                                <div className="settings-image-meta">
                                  <span className="field-hint">
                                    {buildReplyImageLabel(image, index)}
                                  </span>
                                </div>
                                {previewable ? (
                                  <div className="settings-image-preview-frame">
                                    <img
                                      className="settings-image-preview"
                                      src={image.trim()}
                                      alt={`阶段图片预览 ${index + 1}`}
                                    />
                                  </div>
                                ) : null}
                              </div>
                            )
                          })}
                        </div>
                      ) : (
                        <div className="settings-empty">
                          <MessageSquare size={20} />
                          <span>当前阶段未附带图片。</span>
                        </div>
                      )}
                    </div>
                  </div>
                </Card.Content>
              </Card>
            </>
          ) : null}

          {selectedMenu === 'webhooks' ? (
            <>
              {renderGroupCard()}

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>Webhook 标签映射</Card.Title>
                </Card.Header>
                <Card.Content>
                  <MappingListEditor
                    label="Webhook 标签"
                    leftPlaceholder="Webhook"
                    rightPlaceholder="默认标签"
                    entries={settings.webhook_tag_map}
                    onAdd={() => addMapping('webhook_tag_map')}
                    onChange={(index, key, value) =>
                      updateMapping('webhook_tag_map', index, key, value)
                    }
                    onRemove={(index) => removeMapping('webhook_tag_map', index)}
                  />
                </Card.Content>
              </Card>
            </>
          ) : null}

          {selectedMenu === 'variables' ? (
            <>
              {renderGroupCard()}

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>变量说明</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="settings-variable-list">
                    {settings.variables.map((variable) => (
                      <div key={variable.key} className="settings-variable-card">
                        <div className="settings-variable-meta">
                          <strong>{variable.label}</strong>
                          <code>{variable.example}</code>
                        </div>
                        <p>{variable.description}</p>
                      </div>
                    ))}
                  </div>
                </Card.Content>
              </Card>
            </>
          ) : null}
        </div>
      </section>
    </div>
  )
}
