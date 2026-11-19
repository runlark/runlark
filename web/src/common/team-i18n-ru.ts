import type { TeamStrings } from "./team-i18n";

export const TEAM_RU: TeamStrings = {
  teamHeading: "Командная работа",
  teamSubtitle: "Общие workspace, роли, раннеры, offline-черновики и разбор конфликтов",
  ownershipSolo: "Локальный Solo",
  ownershipTeam: (wid, epoch) => `Команда · ${wid} · эпоха ${String(epoch)}`,
  syncOnline: (cursor) => `Онлайн · курсор #${String(cursor)}`,
  syncOffline: "Офлайн · авторитетные изменения заблокированы",
  syncReconnecting: "Синхронизация с сервером команды...",
  syncConflict: "Конфликт (409) · требуется проверка",
  syncNowBtn: "Синхронизировать",
  reconnectBtn: "Переподключить и синхронизировать",
  draftsSummaryChip: (u, c) =>
    c > 0
      ? `Черновиков: ${String(u)} · конфликтов: ${String(c)}`
      : `Offline-черновиков: ${String(u)}`,
  openTeamQueueBtn: "Открыть очередь",
  offlineGateNotice:
    "Сервер команды недоступен. Авторитетные переходы заблокированы; комментарии, новые задачи и правки плана можно сохранить как offline-черновики.",
  conflictBannerTitle: (rev) =>
    rev !== null
      ? `Конфликт ревизий (409) — актуальная ревизия на сервере: ${String(rev)}. Синхронизируйте и повторите.`
      : "Конфликт (409) — обновите состояние сервера перед повтором.",
  serverConnectionTitle: "Сервер команды и приглашения",
  serverUrlLabel: "URL сервера",
  bearerTokenLabel: "Bearer-токен",
  workspaceIdOptionalLabel: "ID workspace (необязательно)",
  connectServerBtn: "Подключить сервер",
  connectedSessionsHeading: "Подключённые сессии",
  noConnectedSessions: "Сессия командного сервера ещё не настроена.",
  acceptInviteHeading: "Присоединиться по токену приглашения",
  inviteTokenLabel: "Одноразовый токен приглашения",
  displayNameLabel: "Отображаемое имя",
  userIdOptionalLabel: "ID пользователя (необязательно)",
  acceptInviteBtn: "Принять приглашение",
  workspaceSelectorHeading: "Рабочие пространства",
  activeWorkspaceLabel: "Активный workspace",
  createWorkspaceHeading: "Создать workspace",
  workspaceNameLabel: "Название workspace",
  workspaceSlugLabel: "Slug",
  createWorkspaceBtn: "Создать workspace",
  membersHeading: (n) => `Участники (${String(n)})`,
  membersEmpty: "Участники workspace ещё не загружены.",
  roleSelectLabel: (name) => `Роль в workspace для ${name}`,
  roleManageDeniedHint: "Изменять роли могут только Owner и Maintainer.",
  lastOwnerHint: "Нельзя понизить роль единственного Owner.",
  invitationsHeading: (n) => `Приглашения (${String(n)})`,
  invitationsEmpty: "Для этого workspace приглашений пока нет.",
  inviteeHintLabel: "Email или логин приглашаемого",
  inviteRoleLabel: "Роль",
  createInviteBtn: "Создать приглашение",
  oneTimeTokenWarning: "Показывается один раз — скопируйте и передайте безопасно:",
  revokeInviteBtn: "Отозвать",
  invitationsForbiddenHint: "Просматривать и отзывать приглашения могут только Owner и Maintainer.",
  runnersHeading: (n) => `Раннеры workspace (${String(n)})`,
  runnersEmpty: "В этом workspace ещё нет зарегистрированных раннеров.",
  registerRunnerHeading: "Зарегистрировать локальный раннер",
  runnerNameLabel: "Имя раннера",
  registerRunnerBtn: "Зарегистрировать раннер",
  localRunnerSummary: (id, name, buffered) =>
    `Локальный раннер: ${name} (${id}) · буфер событий: ${String(buffered)}`,
  noLocalRunner: "Этот демон ещё не зарегистрирован как раннер workspace.",
  draftsHeading: (n) => `Offline-черновики и конфликты (${String(n)})`,
  draftsEmpty: "Неотправленных offline-черновиков и конфликтов нет.",
  draftStateLocal: "Локальный черновик",
  draftStatePending: "Ожидает синхронизации",
  draftStateConflict: (rev) =>
    rev !== null ? `Конфликт (409 · ревизия сервера ${String(rev)})` : "Конфликт (409)",
  draftStateSynced: "Синхронизирован",
  draftExpectedRevLabel: "Ожидаемая ревизия",
  draftPayloadLabel: "Данные черновика (JSON)",
  draftRetryBtn: "Обновить и повторить",
  draftDiscardBtn: "Удалить черновик",
  discardDraftDialogTitle: "Удалить offline-черновик?",
  discardDraftDialogBody:
    "Черновик и связанная запись outbox будут безвозвратно удалены из локального SQLite.",
  discardDraftConfirmBtn: "Удалить навсегда",
  promoteToTeamBtn: "Перевести в Team",
  promoteDialogTitle: "Перевод Solo-проекта в Team",
  promoteWorkspaceLabel: "Целевой ID workspace",
  promoteBlockedNotice: (n) =>
    `Перевод заблокирован: активных или незавершённых запусков — ${String(n)}. Завершите или остановите их перед сменой владельца.`,
  promoteSanitizeNotice:
    "Локальные пути репозитория очищаются перед отправкой. После подтверждения сервера источником истины становится командный сервер.",
  promoteConfirmBtn: "Перевести проект",
  runnerOwnershipHeading: "Назначение раннера и fencing",
  runnerUnassignedLabel: "Не назначен · ожидает онлайн-раннер",
  runnerExclusiveOwner: (label, epoch) =>
    `Единоличный владелец: ${label} · эпоха аренды #${String(epoch)}`,
  noAutoTakeoverNotice: "Автоматический перехват активного запуска запрещён.",
  assignRunnerSelectLabel: "Выбрать онлайн-раннер",
  assignRunnerBtn: "Назначить раннер",
  noOnlineRunnersHint: "В этом workspace нет раннеров в статусе Online.",
  runnerUnreachableBadge: "Раннер недоступен · нет авто-перехвата",
  cancelUnreachableBtn: "Отменить недоступный запуск",
  cancelUnreachableDialogTitle: "Отменить запуск недоступного раннера?",
  cancelUnreachableDialogBody: (label, epoch) =>
    `Раннер ${label} (эпоха аренды #${String(epoch)}) недоступен. Запуск будет явно отменён на сервере без скрытого перехвата другим раннером.`,
  cancelUnreachableReasonLabel: "Причина отмены",
  cancelUnreachableReasonRequired: "Укажите причину отмены недоступного запуска.",
  cancelUnreachableConfirmBtn: "Подтвердить отмену запуска",
  commentsHeading: (n) => `Комментарии append-only (${String(n)})`,
  commentsEmpty: "Комментариев к этой задаче пока нет.",
  commentsImmutableHint: "Комментарии неизменяемы и сохраняются в общей истории.",
  commentBodyLabel: "Новый комментарий",
  commentSubmitBtn: "Отправить комментарий",
  commentQueuedOfflineNotice:
    "Сервер команды недоступен — комментарий сохранён как offline-черновик и поставлен в очередь синхронизации.",
  commentDraftsHeading: "Ожидающие / конфликтные черновики комментариев",
  saveTaskOfflineDraftBtn: "Сохранить как offline-черновик",
  savePlanOfflineDraftBtn: "Сохранить правку плана как offline-черновик",
  casInspectBtn: "Проверить SHA-256",
  casInspectingBtn: "Проверка SHA-256...",
  casVerifiedBadge: "SHA-256 проверен",
  casMissingBadge: "Не синхронизирован локально · неизвестно",
  casCorruptedBadge: "Повреждён · несовпадение SHA-256",
  casTruncatedNotice: "Предпросмотр ограничен первыми 32 КиБ.",
  mobileAcceptanceHeading: "Мобильная привязанная приёмка",
  mobileAcceptanceOfflineHint:
    "Сервер команды офлайн — для приёмки требуется подтверждение сервера.",
  mobileAcceptDialogTitle: "Подтвердить привязанную приёмку",
  mobileAcceptConfirmBtn: "Подтвердить приёмку",
  mobileRejectDialogTitle: "Вернуть на доработку",
  mobileRevisionPlanLabel: (rev, planVer) => `рев. ${String(rev)} · план v${String(planVer)}`,
  formatAnnouncement: (a) => {
    if (!a) return "";
    switch (a.kind) {
      case "synced":
        return `Синхронизировано событий: ${String(a.eventsApplied)}, отправлено из очереди: ${String(a.outboxAcked)}`;
      case "logged_in":
        return `Подключено как ${a.displayName}`;
      case "workspace_created":
        return `Создано пространство ${a.name}`;
      case "role_updated":
        return `Роль ${a.userId} изменена на ${a.role}`;
      case "invitation_created":
        return `Создано приглашение для ${a.inviteeHint}`;
      case "invitation_revoked":
        return `Приглашение ${a.inviteId} отозвано`;
      case "invitation_accepted":
        return `Вы присоединились к пространству ${a.workspaceId}`;
      case "project_promoted":
        return `Проект ${a.projectId} переведён в пространство ${a.workspaceId}`;
      case "runner_registered":
        return `Раннер ${a.name} зарегистрирован`;
      case "runner_assigned":
        return `Назначен раннер ${a.runnerId} (эпоха ${String(a.leaseEpoch)})`;
      case "run_cancelled_unreachable":
        return `Недоступный запуск ${a.runId} отменён`;
      case "comment_queued_offline":
        return "Комментарий поставлен в офлайн-очередь";
      case "comment_posted":
        return "Комментарий опубликован";
      case "draft_saved":
        return `Сохранён офлайн-черновик ${a.draftId}`;
      case "draft_synced":
        return `Черновик ${a.draftId} синхронизирован`;
      case "draft_discarded":
        return `Черновик ${a.draftId} удалён`;
    }
  },
  fieldRequiredError: "Поле обязательно для заполнения.",
  cancelBtn: "Отмена",
  roleLabel: (role) => {
    switch (role) {
      case "owner":
        return "Владелец";
      case "maintainer":
        return "Мейнтейнер";
      case "member":
        return "Участник";
      case "viewer":
        return "Наблюдатель";
    }
  },
  inviteStatusLabel: (status) => {
    switch (status) {
      case "pending":
        return "Ожидает";
      case "accepted":
        return "Принято";
      case "revoked":
        return "Отозвано";
      case "expired":
        return "Истекло";
    }
  },
  runnerStatusLabel: (status) => {
    switch (status) {
      case "online":
        return "Онлайн";
      case "unreachable":
        return "Недоступен";
      case "drained":
        return "Остановлен";
    }
  },
  draftKindLabel: (kind) => {
    switch (kind) {
      case "task_create":
        return "Новая задача";
      case "task_comment":
        return "Комментарий";
      case "plan_edit":
        return "Правка плана";
      case "decision_intent":
        return "Намерение решения";
    }
  },
  conflictCodeHint: (code, fallback) => {
    switch (code) {
      case "solo_to_team_blocked":
        return "Перевод заблокирован: завершите или остановите все активные запуски перед переводом в Team.";
      case "active_run_prevents_takeover":
        return "Активный запуск закреплён за другим раннером; автоматический перехват запрещён.";
      case "revision_mismatch":
        return "Ревизия на сервере изменилась; синхронизируйтесь и обновите ревизию перед повтором.";
      case "last_owner_protected":
        return "Нельзя понизить последнего владельца workspace.";
      case "runner_not_online":
        return "Назначить можно только раннер в статусе Online.";
      case "evidence_required":
        return "Для приёмки требуется проверенный пакет доказательств (evidence_bundle).";
      default:
        return fallback;
    }
  },
};
