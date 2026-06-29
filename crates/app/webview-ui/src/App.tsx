import { type FormEvent, type ReactNode, useEffect, useState } from 'react'
import { Button, Card, Input, Spinner, Toast } from '@heroui/react'
import {
  BarChart3,
  Eye,
  FileText,
  LayoutGrid,
  LogOut,
  Send,
  ShieldCheck,
} from 'lucide-react'
import { api } from './api/client'
import { MeResponse } from './api/types'
import { REVIEW_STAGE_OPTIONS, type ToastKind, type ViewKey, showToast } from './shared'
import { PostsView } from './views/PostsView'
import { AgentView, SettingsView } from './views/SettingsView'
import { TagMappingView } from './views/TagMappingView'
import { StatsView } from './views/StatsView'

function App() {
  const [me, setMe] = useState<MeResponse | null>(null)
  const [authChecked, setAuthChecked] = useState(false)
  const [view, setView] = useState<ViewKey>('review')

  useEffect(() => {
    api<MeResponse>('/auth/me')
      .then(setMe)
      .catch(() => setMe(null))
      .finally(() => setAuthChecked(true))
  }, [])

  async function logout() {
    await api('/auth/logout', { method: 'POST' }).catch(() => undefined)
    setMe(null)
  }

  const notify = (kind: ToastKind, text: string) => showToast(kind, text)

  if (!authChecked) {
    return (
      <HeroShell>
        <div className="boot">
          <Spinner />
        </div>
      </HeroShell>
    )
  }

  if (!me) {
    return (
      <HeroShell>
        <LoginView onAuthed={setMe} notify={notify} />
      </HeroShell>
    )
  }

  return (
    <HeroShell>
      <div className="app-shell">
        <aside className="sidebar">
          <Brand />
          <nav className="nav" aria-label="主导航">
            <Button
              className="nav-button"
              variant={view === 'review' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('review')}
            >
              <Eye size={18} />
              审核
            </Button>
            <Button
              className="nav-button"
              variant={view === 'sent' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('sent')}
            >
              <Send size={18} />
              已发送
            </Button>
            <Button
              className="nav-button"
              variant={view === 'stats' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('stats')}
            >
              <BarChart3 size={18} />
              统计
            </Button>
            {me.role === 'global_admin' ? (
              <Button
                className="nav-button"
                variant={view === 'agent' ? 'primary' : 'tertiary'}
                fullWidth
                onClick={() => setView('agent')}
              >
                <FileText size={18} />
                Agent
              </Button>
            ) : null}
            <Button
              className="nav-button"
              variant={view === 'settings' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('settings')}
            >
              <Send size={18} />
              设置
            </Button>
            <Button
              className="nav-button"
              variant={view === 'tag-mapping' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('tag-mapping')}
            >
              <LayoutGrid size={18} />
              标签映射
            </Button>
          </nav>
          <Card className="account-card" variant="secondary">
            <Card.Content>
              <div className="account-name">{me.username}</div>
              <div className="account-role">
                {me.role === 'global_admin' ? '全局管理员' : me.groups.join(', ')}
              </div>
              <Button size="sm" variant="secondary" fullWidth onClick={logout}>
                <LogOut size={16} />
                退出
              </Button>
            </Card.Content>
          </Card>
        </aside>

        <main className="main">
          {view === 'review' ? (
            <PostsView
              notify={notify}
              mode="review"
              title="稿件审核"
              description="选择左侧稿件后可在右侧处理详情"
              emptyText="没有符合条件的稿件"
              stageOptions={REVIEW_STAGE_OPTIONS}
              initialStage="__active__"
              allowSelection
              allowActions
              allowBatchActions
              allowOnlyActionable
              allowOnlyError
              allowStageFilter
              allowDetailActions
            />
          ) : view === 'sent' ? (
            <PostsView
              notify={notify}
              mode="sent"
              title="已发送"
              description="这里只展示已经发送完成的稿件，方便单独查看"
              emptyText="没有已发送的稿件"
              stageOptions={[{ value: 'sent', label: '已发送' }]}
              initialStage="sent"
              allowSelection={false}
              allowActions={false}
              allowBatchActions={false}
              allowOnlyActionable={false}
              allowOnlyError={false}
              allowStageFilter={false}
              allowDetailActions={false}
            />
          ) : view === 'stats' ? (
            <StatsView notify={notify} />
          ) : view === 'agent' && me.role === 'global_admin' ? (
            <AgentView notify={notify} />
          ) : view === 'tag-mapping' ? (
            <TagMappingView notify={notify} />
          ) : (
            <SettingsView me={me} notify={notify} />
          )}
        </main>
      </div>
    </HeroShell>
  )
}

function HeroShell({ children }: { children: ReactNode }) {
  return (
    <>
      {children}
      <Toast.Provider placement="bottom end" />
    </>
  )
}

function Brand({ large = false }: { large?: boolean }) {
  return (
    <div className={large ? 'brand brand-large' : 'brand'}>
      <div>
        <strong>OQQWall</strong>
        <span>审核后台</span>
      </div>
    </div>
  )
}

function LoginView({
  onAuthed,
  notify,
}: {
  onAuthed: (me: MeResponse) => void
  notify: (kind: ToastKind, text: string) => void
}) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [loading, setLoading] = useState(false)

  async function submit(event: FormEvent) {
    event.preventDefault()
    setLoading(true)
    try {
      const result = await api<MeResponse>('/auth/login', {
        method: 'POST',
        body: JSON.stringify({ username, password }),
      })
      onAuthed(result)
      notify('success', '登录成功')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="login-page">
      <Card className="login-card">
        <Card.Content>
          <form className="login-form" onSubmit={submit}>
            <Brand large />
            <Input
              placeholder="用户名"
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              autoComplete="username"
            />
            <Input
              placeholder="密码"
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              autoComplete="current-password"
            />
            <Button type="submit" fullWidth isDisabled={loading || !username || !password}>
              {loading ? <Spinner size="sm" /> : <ShieldCheck size={16} />}
              登录
            </Button>
          </form>
        </Card.Content>
      </Card>
    </div>
  )
}

export default App
