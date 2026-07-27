import type { en } from "./en";

type TranslationShape<T> = {
	[K in keyof T]: T[K] extends string ? string : TranslationShape<T[K]>;
};

export const ja = {
	nav: {
		sms: "SMS",
		modem: "モデム",
		forwarding: "転送",
		config: "設定",
	},
	header: {
		ariaPrimary: "メインナビゲーション",
		ariaBackConfig: "設定に戻る",
		ariaConfigCategories: "設定カテゴリ",
		ariaUnsavedChanges: "未保存の変更",
	},
	common: {
		refresh: "更新",
		cancel: "キャンセル",
		save: "保存",
		check: "チェック",
		restart: "再起動",
		retry: "再試行",
		done: "完了",
		search: "検索",
		add: "追加",
		remove: "削除",
	},
	modem: {
		title: "モデム",
		lastChecked: "最終確認 {{time}}",
		statusUnavailable: "ステータスを利用できません",
		loading: "モデムのステータスを読み込んでいます…",
		refresh: "更新",
		field: {
			configuredPath: "設定パス",
			resolvedModem: "解決済みモデム",
			enabled: "有効",
			state: "ステータス",
			sim: "SIM",
			phoneNumber: "電話番号",
			operator: "オペレーター",
			signal: "信号",
			access: "アクセス技術",
			messaging: "メッセージング",
			mmcli: "mmcli",
		},
		value: {
			yes: "はい",
			no: "いいえ",
			unknown: "不明",
			notFound: "見つかりません",
			notReported: "報告なし",
			available: "利用可能",
			unavailable: "利用不可",
			missing: "不足",
			notSelected: "未選択",
		},
		status: {
			ok: "OK",
			degraded: "低下",
			error: "エラー",
			unknown: "不明",
		},
		smsOverIms: {
			title: "SMS over IMS",
			description:
				"モデムから報告されたものであり、各メッセージが実際に使用したルートを証明するものではありません。",
			field: {
				configured: "設定済み",
				registration: "登録ステータス",
				smsService: "SMSサービス",
				technology: "技術",
				qmicli: "qmicli",
				qmiDevice: "QMIデバイス",
				evidence: "証拠",
			},
			enum: {
				enabled: "有効",
				disabled: "無効",
				registered: "登録済み",
				registering: "登録中",
				limited: "制限あり",
				notRegistered: "未登録",
				notAvailable: "利用不可",
				available: "利用可能",
				unknown: "不明",
			},
			availableOverWlan: "WLANで利用可能",
			technology: {
				wwan: "WWAN",
				wlan: "WLAN",
				interworkingWlan: "Interworking WLAN",
				unknown: "不明",
			},
			evidence: {
				qmiImsa: "QMI IMSA",
				qmiIms: "QMI IMS",
				noEvidence: "ランタイム証拠なし",
			},
		},
		actions: {
			enable: "有効にする",
			disable: "無効にする",
		},
		dangerZone: {
			title: "危険な操作",
			reset: "モデムをリセット",
			dialogTitle: "モデムをリセットしますか？",
			dialogDescription:
				"これにより蜂窝通信が切断され、再列挙中にモデムが一時的に消失する可能性があります。",
			cancel: "キャンセル",
			confirmReset: "リセット",
		},
		diagnostics: {
			title: "診断",
			reasons: "理由：{{reasons}}",
			possibleNewPath: "可能性のある新しいモデムパス：{{path}}",
			error: "エラー：{{error}}",
		},
		imsDiagnostics: {
			imsProbeNotAttempted: "IMSプローブは試行されませんでした。",
			modemNotResolved:
				"モデムが解決されなかったため、IMSプローブはスキップされました。",
			modemDisabled:
				"モデムが無効になっているため、IMSプローブはスキップされました。",
			qmicliMissing: "qmicliがインストールされていないか、実行できません。",
			qmicliPathInvalid: "設定されたqmicliパスが無効です。",
			qmicliProbeFailed: "qmicliの機能検出に失敗しました。",
			imsProbePermissionDenied:
				"権限のためqmicliを実行できませんでした。",
			qmiPortUnavailable:
				"ModemManagerがQMI制御ポートを報告しませんでした。",
			qmiPortAmbiguous: "複数のQMI制御ポートが報告されました。",
			qmiProxyUnavailable: "QMIプロキシが利用できません。",
			imsProbeTimeout: "IMSプローブが時間制限を超えました。",
			imsServicesQueryFailed: "IMSサービスクエリに失敗しました。",
			imsServicesQueryUnavailable:
				"qmicliはIMSサービスクエリを公開していません。",
			imsRegistrationQueryFailed: "IMS登録クエリに失敗しました。",
			imsRegistrationQueryUnavailable:
				"qmicliはIMS登録クエリを公開していません。",
			imsSettingsQueryFailed: "IMS設定クエリに失敗しました。",
			imsSettingsQueryUnavailable:
				"qmicliはIMS設定クエリを公開していません。",
			imsServicesOutputUnrecognized:
				"IMSサービス応答を認識できませんでした。",
			imsRegistrationOutputUnrecognized:
				"IMS登録応答を認識できませんでした。",
			imsSettingsOutputUnrecognized:
				"IMS設定応答を認識できませんでした。",
			imsServicesOutputNonstandard:
				"IMSサービス応答が非標準のラベルを使用しました。",
			imsRegistrationOutputNonstandard:
				"IMS登録応答が非標準のラベルを使用しました。",
			imsSettingsOutputNonstandard:
				"IMS設定応答が非標準のラベルを使用しました。",
			imsStateInconsistent:
				"モデムが報告したIMS設定とランタイム状態が矛盾しています。",
			fallback: "追加のIMS診断情報は利用できません。",
		},
	},
	forwarding: {
		sidebar: {
			title: "転送",
			operations: "操作",
			ariaLabel: "転送プロファイル",
			ariaViews: "転送ビュー",
			ariaRefresh: "転送ステータスを更新",
			ariaClose: "転送ナビゲーションを閉じる",
			ariaOpen: "転送ナビゲーションを開く",
		},
		overview: {
			title: "概要",
			subtitle: "すべてのプロファイルのスナップショット",
			configured: "設定済み",
			historical: "履歴",
			noConfiguredProfiles: "プロファイルが設定されていません",
			noHistoricalProfiles: "保持された履歴プロファイルはありません",
		},
		snapshot: {
			generated: "スナップショット生成",
			generatedDescription: "プロファイルごとに最大{{limit}}件の試行を保持",
		},
		loading: "転送ステータスを読み込んでいます…",
		error: {
			title: "転送ステータスを読み込めません",
			description: "転送スナップショットを読み込めませんでした。",
			refreshFailed: "更新に失敗しました",
			refreshDescription: "以前のスナップショットを表示しています。{{error}}",
		},
		srLive: {
			refreshing: "転送ステータスを更新しています。",
			snapshot: "転送スナップショットは{{time}}に生成されました。",
		},
		detail: {
			overview: "概要",
			overviewSubtitle: "設定済みプロファイルと保持された試行履歴",
			retainedAttempts: "保持された転送試行",
			notPresent: "最新のスナップショットに存在しません",
			lastUpdated: "最終更新 {{time}}",
		},
		overviewSection: {
			currentSnapshot: "現在のスナップショット",
			coverage: "転送カバレッジ",
			description:
				"最新のバックエンドスナップショットからの設定ステータスと保持された試行の可用性。",
			configuredProfiles: "設定済みプロファイル",
			enabledProfiles: "有効なプロファイル",
			profilesWithAttempts: "試行が保持されているプロファイル",
			profileSnapshot: "プロファイルスナップショット",
			profileSnapshotDescription:
				"設定済みまたは履歴プロファイルごとの最新の保持結果。",
			empty: "転送プロファイルが設定されていません。",
			emptyDescription:
				"保持された履歴プロファイルの試行も利用できません。",
		},
		table: {
			ariaLabel: "転送プロファイルスナップショット",
			profile: "プロファイル",
			state: "ステータス",
			latestOutcome: "最新の結果",
			latestCompleted: "最新の完了",
			retained: "保持数",
			attempt: "試行",
			completed: "完了",
			outcome: "結果",
			timing: "タイミング",
			error: "エラー",
		},
		profile: {
			unavailable: "プロファイルを利用できません",
			unavailableDescription:
				"プロファイル{{key}}は最新の転送スナップショットに存在しません。",
			viewOverview: "概要を表示",
		},
		badge: {
			configured: "設定済み",
			enabled: "有効",
			disabled: "無効",
			historical: "履歴",
			retry: "再試行",
		},
		outcome: {
			success: "成功",
			transientFailure: "一時的な失敗",
			permanentFailure: "永続的な失敗",
			unknown: "結果不明",
			noAttempts: "試行なし",
			latest: "最新：{{outcome}}",
		},
		attempts: {
			title_one: "最新{{count}}件の試行",
			title_other: "最新{{count}}件の試行",
			retainedDescription: "このスナップショットに{{count}}件保持",
			newestFirst: "新しい順",
			empty: "転送試行はまだありません。",
			emptyDescription:
				"このスナップショットには、このプロファイルの保持された試行が含まれていません。",
			label: "{{key}}の{{label}}",
		},
		mobile: {
			completed: "完了",
			timing: "タイミング",
			error: "エラー",
			retained: "{{count}}件保持",
		},
		timing: {
			dispatch: "ディスパッチ {{time}}",
			request: "リクエスト {{time}}",
		},
	},
	messages: {
		title: "メッセージ",
		sim: "SIM {{number}}",
		aria: {
			newMessage: "新規メッセージ",
			filters: "フィルター",
			backConversations: "会話一覧に戻る",
			markConversationRead: "会話を既読にする",
			messageTimeline: "メッセージタイムライン",
			conversationActions: "会話操作",
			sendMessage: "メッセージを送信",
			searchMessages: "メッセージを検索",
		},
		search: {
			placeholder: "メッセージを検索",
		},
		filter: {
			title: "メッセージツール",
			description: "受信トレイをフィルターするか、現在のメッセージビューをエクスポートします。",
			direction: "方向",
			allDirections: "すべての方向",
			inbound: "受信",
			outbound: "送信",
			status: "ステータス",
			allStatuses: "すべてのステータス",
			received: "受信済み",
			sending: "送信中",
			sent: "送信済み",
			failed: "失敗",
			unreadOnly: "未読のみ",
			exportCsv: "CSV",
			exportJson: "JSON",
			done: "完了",
			search: "検索",
		},
		conversationList: {
			empty: "会話がありません",
			emptyDescription: "受信および送信のSMSスレッドがここに表示されます。",
			messages: "{{count}}件のメッセージ",
			noMatching: "一致するメッセージがありません",
			noMatchingDescription: "フィルターを調整するか、次のSMSイベントを待ってください。",
		},
		thread: {
			loadingOlder: "古いメッセージを読み込んでいます",
			loadOlder: "古いメッセージを読み込む",
			newMessage: "新規メッセージ",
			newMessageSubtitle: "受信者を選択してSMSを作成します",
			selectConversation: "会話を選択",
			selectConversationSubtitle: "一覧からスレッドを選択します",
			noThreadSelected: "スレッドが選択されていません",
			noThreadDescription: "会話を選択するか、新規SMSを開始してください。",
			recipientLabel: "宛先",
			recipientPlaceholder: "電話番号",
			composerPlaceholder: "メッセージ",
			sendMessage: "送信",
			sendingMessage: "送信中…",
		},
		direction: {
			sent: "送信済み",
			inbox: "受信トレイ",
			failed: "失敗",
		},
		actions: {
			selectMessages: "メッセージを選択",
			stopSelecting: "選択を停止",
			markRead: "既読にする ({{count}})",
			markUnread: "未読にする ({{count}})",
			deleteSelected: "選択項目を削除",
			markConversationRead: "会話を既読にする",
			conversationActions: "会話操作",
		},
		relativeDay: {
			today: "今日",
			yesterday: "昨日",
			daysAgo: "{{count}}日前",
		},
	},
	config: {
		sidebar: {
			title: "設定",
			ariaLabel: "設定カテゴリ",
			ariaUnsaved: "未保存の変更",
			categories: "カテゴリ",
			dirty: "{{count}}件の{{category}}が変更されました",
			dirty_one: "{{count}}件のカテゴリが変更されました",
			dirty_other: "{{count}}件のカテゴリが変更されました",
			clean: "未保存の変更はありません",
		},
		editor: {
			unsavedDraft: "未保存の下書き",
			saved: "設定を保存しました",
			restartRequired: "再起動が必要です",
			loading: "設定を読み込んでいます…",
		},
		error: {
			title: "設定を利用できません",
		},
		action: {
			save: "保存",
			check: "チェック",
			restart: "再起動",
			checking: "完全な下書きをチェックしています…",
			notChecked: "未チェック",
			checkPassed: "チェック合格",
			checkFailed: "チェック失敗：{{message}}",
		},
		status: {
			saved: "設定が保存されました。",
			savedRestart: "設定が保存されました。再起動が必要です。",
			restartScheduled:
				"再起動がスケジュールされました。ダッシュボードが一時的に切断される場合があります。",
			restartFailed: "再起動に失敗しました：{{message}}",
		},
		restartDialog: {
			title: "サービスの再起動をスケジュールしますか？",
			description:
				"このリクエストはservice-managerコマンドをスケジュールするだけです。サービスが再び利用可能になる前に、このページが切断される場合があります。",
			unsavedWarning:
				"未保存の編集はこのブラウザタブにのみ存在します。再起動は永続化されたファイルを使用するため、この下書きが復元できなくなる可能性があります。",
			cancel: "キャンセル",
			scheduleRestart: "再起動をスケジュール",
		},
		saveReview: {
			title: "設定の変更を確認",
			description:
				"現在のファイルを置き換える正確なTOMLを確認し、もう一度確認して保存してください。",
			generating: "TOMLの差分を生成し、下書きをチェックしています…",
			conflict: "ディスク上の設定が変更されました",
			previewFailed: "プレビューに失敗しました",
			reload: "ディスクから再読み込みして下書きを破棄",
			checkPassed: "チェック合格",
			checkFailed: "チェック失敗",
			securityWarning:
				"この差分は意図的にマスクされていません。パスワード、トークン、Webhook URL、その他の認証情報がこの認証済みダイアログとネットワーク応答に表示されます。",
			operationalWarnings: "運用上の警告",
			tomlDiff: "TOML差分",
			noChanges: "保存するファイルの変更はありません。",
			saveFailed: "保存に失敗しました：{{error}}",
			noRuntimeChange: "ランタイムの変更なし",
			restartRequired: "再起動が必要です",
			cancel: "キャンセル",
			saveConfig: "設定を保存",
			saveAndRestart: "保存して再起動をスケジュール",
		},
		warnings: {
			passwordChange:
				"保存と再起動のスケジュール後、すべてのセッションからサインアウトされます。",
			apiDisable: "再起動後、ダッシュボードは利用できなくなります。",
			apiEndpointChange:
				"再起動後、ダッシュボードのアドレスが変更される場合があります。",
			databasePathChange:
				"再起動後、サービスは異なるメッセージデータベースを使用します。",
		},
		leaveDialog: {
			title: "未保存の変更を破棄して離れますか？",
			description:
				"設定の下書きには認証情報が含まれており、意図的にブラウザに保存されていません。離れると破棄されます。",
			stay: "留まる",
			discard: "破棄して離れる",
		},
		sections: {
			device: "デバイス",
			deviceDescription: "モデムの識別子とオブジェクトパス",
			sms: "SMS",
			smsDescription: "ストレージフィルターとコードキーワード",
			forwarding: "転送",
			forwardingDescription: "配信ワーカーとチャネルプロファイル",
			api: "Web API",
			apiDescription: "ダッシュボードのアクセスと永続化",
			timeouts: "タイムアウト",
			timeoutsDescription: "HTTPとシェル実行の制限",
			retention: "保持",
			retentionDescription: "自動メッセージクリーンアップ",
		},
		fields: {
			device: {
				sectionTitle: "デバイス",
				sectionDescription:
					"このリレーを識別し、メッセージの送受信を行うModemManagerオブジェクトを選択します。",
				deviceName: "デバイス名",
				deviceNameDescription:
					"転送ペイロードに含まれており、下流チャネルがソースを識別できるようにします。",
				modemPath: "モデムオブジェクトパス",
				modemPathDescription:
					"/org/freedesktop/ModemManager1/Modem/ の下のModemManagerパスである必要があります。",
			},
			sms: {
				sectionTitle: "SMS",
				sectionDescription:
					"無視するモデムのストレージ場所と、確認コードメッセージを識別するフレーズを制御します。",
				ignoredStorage: "無視するストレージ",
				ignoredStorageDescription: "カンマ区切りのストレージ識別子（例：sm）。",
				codeKeywords: "コードキーワード",
				codeKeywordsDescription:
					"カンマ区切りで大文字小文字を区別しないフレーズ。確認コードの認識に使用されます。",
			},
			forwarding: {
				sectionTitle: "転送",
				sectionDescription:
					"配信の同時実行性、チャネル認証情報、および受信メッセージを受ける名前付きプロファイルを構成します。",
				concurrency: "同時配信数",
				concurrencyDescription: "一度に処理される転送ジョブの数。有効範囲：1–16。",
			},
			api: {
				sectionTitle: "Web API",
				sectionDescription:
					"ダッシュボードの可用性、リスナーアドレス、認証、およびメッセージデータベースを制御します。",
				enableApi: "Web APIを有効にする",
				enableApiDescription:
					"APIを無効にすると、再起動後にこのダッシュボードへのアクセスが削除されます。",
				bindAddress: "バインドアドレス",
				port: "ポート",
				portDescription: "有効範囲：1–65535。",
				ipv6: "IPv6コンパニオン",
				ipv6Description:
					"安全なIPv6コンパニオンアドレスを推測できる場合にも、そのアドレスでリッスンします。",
				password: "パスワード",
				passwordDescription:
					"この値を変更すると、保存と再起動のスケジュールが一度に行われ、すべてのセッションからサインアウトされます。",
				databasePath: "データベースパス",
			},
			timeouts: {
				sectionTitle: "タイムアウト",
				sectionDescription:
					"接続の確立、プロバイダーリクエスト、およびシェルプロファイルの実行を制限します。すべての値は秒単位です。",
				connectTimeout: "接続タイムアウト",
				connectTimeoutDescription:
					"正の値であり、リクエストタイムアウト以下である必要があります。",
				requestTimeout: "リクエストタイムアウト",
				shellTimeout: "シェルタイムアウト",
			},
			retention: {
				sectionTitle: "保持",
				sectionDescription:
					"アクティブな配信があるメッセージを保持しながら、古いターミナルメッセージを制限されたバッチで削除します。",
				enableCleanup: "クリーンアップを有効にする",
				maxAge: "最大経過日数",
				maxAgeDescription: "この日数より古いメッセージがクリーンアップの対象となります。",
				batchSize: "バッチサイズ",
				batchSizeDescription: "1回のクリーンアップで削除される最大行数。",
			},
		},
		channel: {
			deliveryRoutes: "配信ルート",
			deliveryRoutesDescription: "転送メッセージを受信するプロファイルを有効にします。",
			profilesActive: "{{enabled}} / {{total}} アクティブ",
			missingProfiles: "不足している転送プロファイル",
			missingProfilesDescription:
				"これらの有効な参照は、設定済みのプロファイルと一致しません。この設定を有効にするには、それらを削除してください。",
			removeReference: "参照を削除",
			removeReferenceAria: "不足している転送参照{{ref}}を削除",
			noProfiles: "プロファイルなし",
			enabled: "有効",
			disabled: "無効",
			remove: "削除",
			add: "追加",
			profileName: "プロファイル名",
			addProfile: "{{channel}}プロファイルを追加",
			duplicateName: "そのプロファイル名はすでに存在します。",
			enableAria: "{{ref}}の転送を有効にする",
			removeAria: "{{ref}}を削除",
			removeDialog: {
				title: "転送プロファイルを削除しますか？",
				description:
					"現在の下書きからプロファイルの認証情報と有効な参照を削除します。保存するまで変更は書き込まれません。",
				cancel: "キャンセル",
				remove: "プロファイルを削除",
			},
		},
	},
	login: {
		title: "SMS Relayed",
		password: "パスワード",
		login: "ログイン",
		loginFailed: "ログインに失敗しました",
		notice: {
			configSavedRestart:
				"設定が保存され、再起動がスケジュールされました。サービスが復帰したら、新しいパスワードでサインインしてください。",
		},
	},
	phoneCopy: {
		copy: "コピー",
		copied: "コピーしました",
		copyFailed: "コピーに失敗しました",
		ariaLabel: "電話番号をコピー",
		srCopied: "電話番号をコピーしました",
		srFailed: "電話番号のコピーに失敗しました",
	},
	language: {
		label: "言語",
		en: "English",
		zhCN: "简体中文",
		ja: "日本語",
		ko: "한국어",
		fr: "Français",
		es: "Español",
	},
} satisfies TranslationShape<typeof en>;
