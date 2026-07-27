import type { en } from "./en";

type TranslationShape<T> = {
	[K in keyof T]: T[K] extends string ? string : TranslationShape<T[K]>;
};

export const es = {
	nav: {
		sms: "SMS",
		modem: "Módem",
		forwarding: "Reenvío",
		config: "Configuración",
	},
	header: {
		ariaPrimary: "Principal",
		ariaBackConfig: "Volver a la configuración",
		ariaConfigCategories: "Categorías de configuración",
		ariaUnsavedChanges: "Cambios sin guardar",
	},
	common: {
		refresh: "Actualizar",
		cancel: "Cancelar",
		save: "Guardar",
		check: "Comprobar",
		restart: "Reiniciar",
		retry: "Reintentar",
		done: "Listo",
		search: "Buscar",
		add: "Añadir",
		remove: "Eliminar",
	},
	modem: {
		title: "Módem",
		lastChecked: "Última comprobación {{time}}",
		statusUnavailable: "Estado no disponible",
		loading: "Cargando estado del módem…",
		refresh: "Actualizar",
		field: {
			configuredPath: "Ruta configurada",
			resolvedModem: "Módem resuelto",
			enabled: "Habilitado",
			state: "Estado",
			sim: "SIM",
			phoneNumber: "Número de teléfono",
			operator: "Operador",
			signal: "Señal",
			access: "Acceso",
			messaging: "Mensajería",
			mmcli: "mmcli",
		},
		value: {
			yes: "Sí",
			no: "No",
			unknown: "Desconocido",
			notFound: "No encontrado",
			notReported: "No notificado",
			available: "Disponible",
			unavailable: "No disponible",
			missing: "Ausente",
			notSelected: "No seleccionado",
		},
		status: {
			ok: "OK",
			degradated: "DEGRADADO",
			error: "ERROR",
			unknown: "DESCONOCIDO",
		},
		smsOverIms: {
			title: "SMS over IMS",
			description:
				"Notificado por el módem; esto no demuestra la ruta que utiliza cada mensaje.",
			field: {
				configured: "Configurado",
				registration: "Registro",
				smsService: "Servicio de SMS",
				technology: "Tecnología",
				qmicli: "qmicli",
				qmiDevice: "Dispositivo QMI",
				evidence: "Evidencia",
			},
			enum: {
				enabled: "Habilitado",
				disabled: "Deshabilitado",
				registered: "Registrado",
				registering: "Registrando",
				limited: "Limitado",
				notRegistered: "No registrado",
				notAvailable: "No disponible",
				available: "Disponible",
				unknown: "Desconocido",
			},
			availableOverWlan: "Disponible a través de WLAN",
			technology: {
				wwan: "WWAN",
				wlan: "WLAN",
				interworkingWlan: "Interworking WLAN",
				unknown: "Desconocido",
			},
			evidence: {
				qmiImsa: "QMI IMSA",
				qmiIms: "QMI IMS",
				noEvidence: "Sin evidencia en tiempo de ejecución",
			},
		},
		actions: {
			enable: "Habilitar",
			disable: "Deshabilitar",
		},
		dangerZone: {
			title: "Zona de peligro",
			reset: "Restablecer módem",
			dialogTitle: "¿Restablecer el módem?",
			dialogDescription:
				"Esto puede desconectar el servicio celular y hacer que el módem desaparezca mientras se reenumera.",
			cancel: "Cancelar",
			confirmReset: "Restablecer",
		},
		diagnostics: {
			title: "Diagnósticos",
			reasons: "Motivos: {{reasons}}",
			possibleNewPath: "Posible nueva ruta del módem: {{path}}",
			error: "Error: {{error}}",
		},
		imsDiagnostics: {
			imsProbeNotAttempted: "No se intentó el sondeo de IMS.",
			modemNotResolved:
				"Se omitió el sondeo de IMS porque no se resolvió ningún módem.",
			modemDisabled:
				"Se omitió el sondeo de IMS porque el módem está deshabilitado.",
			qmicliMissing: "qmicli no está instalado o no se pudo ejecutar.",
			qmicliPathInvalid: "La ruta de qmicli configurada no es válida.",
			qmicliProbeFailed: "La detección de capacidades de qmicli falló.",
			imsProbePermissionDenied:
				"No se pudo ejecutar qmicli debido a los permisos.",
			qmiPortUnavailable: "ModemManager no notificó ningún puerto de control QMI.",
			qmiPortAmbiguous: "Se notificó más de un puerto de control QMI.",
			qmiProxyUnavailable: "El proxy QMI no está disponible.",
			imsProbeTimeout: "El sondeo de IMS excedió su tiempo límite.",
			imsServicesQueryFailed: "La consulta de servicios de IMS falló.",
			imsServicesQueryUnavailable:
				"qmicli no expone la consulta de servicios de IMS.",
			imsRegistrationQueryFailed: "La consulta de registro de IMS falló.",
			imsRegistrationQueryUnavailable:
				"qmicli no expone la consulta de registro de IMS.",
			imsSettingsQueryFailed: "La consulta de ajustes de IMS falló.",
			imsSettingsQueryUnavailable:
				"qmicli no expone la consulta de ajustes de IMS.",
			imsServicesOutputUnrecognized:
				"La respuesta del servicio de IMS no se reconoció.",
			imsRegistrationOutputUnrecognized:
				"La respuesta de registro de IMS no se reconoció.",
			imsSettingsOutputUnrecognized:
				"La respuesta de ajustes de IMS no se reconoció.",
			imsServicesOutputNonstandard:
				"La respuesta del servicio de IMS utilizó una etiqueta no estándar.",
			imsRegistrationOutputNonstandard:
				"La respuesta de registro de IMS utilizó una etiqueta no estándar.",
			imsSettingsOutputNonstandard:
				"La respuesta de ajustes de IMS utilizó una etiqueta no estándar.",
			imsStateInconsistent:
				"El módem notificó una configuración de IMS y un estado en tiempo de ejecución incoherentes.",
			fallback: "La información de diagnóstico adicional de IMS no está disponible.",
		},
	},
	forwarding: {
		sidebar: {
			title: "Reenvío",
			operations: "Operaciones",
			ariaLabel: "Perfiles de reenvío",
			ariaViews: "Vistas de reenvío",
			ariaRefresh: "Actualizar estado de reenvío",
			ariaClose: "Cerrar navegación de reenvío",
			ariaOpen: "Abrir navegación de reenvío",
		},
		overview: {
			title: "Resumen",
			subtitle: "Todas las instantáneas de perfiles",
			configured: "Configurado",
			historical: "Histórico",
			noConfiguredProfiles: "No hay perfiles configurados",
			noHistoricalProfiles: "No hay perfiles históricos conservados",
		},
		snapshot: {
			generated: "Instantánea generada",
			generatedDescription: "Hasta {{limit}} intentos conservados por perfil",
		},
		loading: "Cargando estado de reenvío…",
		error: {
			title: "No se puede cargar el estado de reenvío",
			description: "No se pudo cargar la instantánea de reenvío.",
			refreshFailed: "Error al actualizar",
			refreshDescription: "Mostrando la instantánea anterior. {{error}}",
		},
		srLive: {
			refreshing: "Actualizando estado de reenvío.",
			snapshot: "Instantánea de reenvío generada {{time}}.",
		},
		detail: {
			overview: "Resumen",
			overviewSubtitle: "Perfiles configurados e historial de intentos conservados",
			retainedAttempts: "Intentos de reenvío conservados",
			notPresent: "No presente en la última instantánea",
			lastUpdated: "Última actualización {{time}}",
		},
		overviewSection: {
			currentSnapshot: "Instantánea actual",
			coverage: "Cobertura de reenvío",
			description:
				"Estado de configuración y disponibilidad de intentos conservados de la última instantánea del backend.",
			configuredProfiles: "Perfiles configurados",
			enabledProfiles: "Perfiles habilitados",
			profilesWithAttempts: "Perfiles con intentos conservados",
			profileSnapshot: "Instantánea de perfil",
			profileSnapshotDescription:
				"Último resultado conservado para cada perfil configurado o histórico.",
			empty: "No hay perfiles de reenvío configurados.",
			emptyDescription:
				"Tampoco hay intentos de perfiles históricos conservados disponibles.",
		},
		table: {
			ariaLabel: "Instantánea de perfiles de reenvío",
			profile: "Perfil",
			state: "Estado",
			latestOutcome: "Último resultado",
			latestCompleted: "Último completado",
			retained: "Conservados",
			attempt: "Intento",
			completed: "Completado",
			outcome: "Resultado",
			timing: "Tiempos",
			error: "Error",
		},
		profile: {
			unavailable: "Perfil no disponible",
			unavailableDescription:
				"El perfil {{key}} no está presente en la última instantánea de reenvío.",
			viewOverview: "Ver resumen",
		},
		badge: {
			configured: "Configurado",
			enabled: "Habilitado",
			disabled: "Deshabilitado",
			historical: "Histórico",
			retry: "Reintentar",
		},
		outcome: {
			success: "Correcto",
			transientFailure: "Fallo transitorio",
			permanentFailure: "Fallo permanente",
			unknown: "Resultado desconocido",
			noAttempts: "Sin intentos",
			latest: "Último: {{outcome}}",
		},
		attempts: {
			title_one: "Último {{count}} intento",
			title_other: "Últimos {{count}} intentos",
			retainedDescription: "{{count}} conservados en esta instantánea",
			newestFirst: "Más recientes primero",
			empty: "Aún no hay intentos de reenvío.",
			emptyDescription:
				"Esta instantánea no contiene intentos conservados para este perfil.",
			label: "{{label}} para {{key}}",
		},
		mobile: {
			completed: "Completado",
			timing: "Tiempos",
			error: "Error",
			retained: "{{count}} conservados",
		},
		timing: {
			dispatch: "Envío {{time}}",
			request: "Solicitud {{time}}",
		},
	},
	messages: {
		title: "Mensajes",
		sim: "SIM {{number}}",
		aria: {
			newMessage: "Mensaje nuevo",
			filters: "Filtros",
			backConversations: "Volver a las conversaciones",
			markConversationRead: "Marcar conversación como leída",
			messageTimeline: "Línea temporal de mensajes",
			conversationActions: "Acciones de conversación",
			sendMessage: "Enviar mensaje",
			searchMessages: "Buscar mensajes",
		},
		search: {
			placeholder: "Buscar mensajes",
		},
		filter: {
			title: "Herramientas de mensajes",
			description: "Filtre la bandeja de entrada o exporte la vista actual de mensajes.",
			direction: "Dirección",
			allDirections: "Todas las direcciones",
			inbound: "Entrante",
			outbound: "Saliente",
			status: "Estado",
			allStatuses: "Todos los estados",
			received: "Recibido",
			sending: "Enviando",
			sent: "Enviado",
			failed: "Fallido",
			unreadOnly: "Solo no leídos",
			exportCsv: "CSV",
			exportJson: "JSON",
			done: "Listo",
			search: "Buscar",
		},
		conversationList: {
			empty: "No hay conversaciones",
			emptyDescription: "Los hilos de SMS entrantes y salientes aparecerán aquí.",
			messages: "{{count}} mensajes",
			noMatching: "No hay mensajes coincidentes",
			noMatchingDescription: "Ajuste los filtros o espere al siguiente evento de SMS.",
		},
		thread: {
			loadingOlder: "Cargando mensajes anteriores",
			loadOlder: "Cargar mensajes anteriores",
			newMessage: "Mensaje nuevo",
			newMessageSubtitle: "Elija un destinatario y escriba un SMS",
			selectConversation: "Seleccionar una conversación",
			selectConversationSubtitle: "Elija un hilo de la lista",
			noThreadSelected: "Ningún hilo seleccionado",
			noThreadDescription: "Elija una conversación o inicie un nuevo SMS.",
			recipientLabel: "Para",
			recipientPlaceholder: "Número de teléfono",
			composerPlaceholder: "Mensaje",
			sendMessage: "Enviar",
			sendingMessage: "Enviando…",
		},
		direction: {
			sent: "Enviado",
			inbox: "Bandeja de entrada",
			failed: "Fallido",
		},
		actions: {
			selectMessages: "Seleccionar mensajes",
			stopSelecting: "Detener selección",
			markRead: "Marcar como leídos ({{count}})",
			markUnread: "Marcar como no leídos ({{count}})",
			deleteSelected: "Eliminar selección",
			markConversationRead: "Marcar conversación como leída",
			conversationActions: "Acciones de conversación",
		},
		relativeDay: {
			today: "Hoy",
			yesterday: "Ayer",
			daysAgo: "Hace {{count}} días",
		},
	},
	config: {
		sidebar: {
			title: "Configuración",
			ariaLabel: "Categorías de configuración",
			ariaUnsaved: "Cambios sin guardar",
			categories: "Categorías",
			dirty: "{{count}} {{category}} modificado",
			dirty_one: "{{count}} categoría modificada",
			dirty_other: "{{count}} categorías modificadas",
			clean: "Sin cambios sin guardar",
		},
		editor: {
			unsavedDraft: "Borrador sin guardar",
			saved: "Configuración guardada",
			restartRequired: "Reinicio necesario",
			loading: "Cargando configuración…",
		},
		error: {
			title: "Configuración no disponible",
		},
		action: {
			save: "Guardar",
			check: "Comprobar",
			restart: "Reiniciar",
			checking: "Comprobando borrador completo…",
			notChecked: "Sin comprobar",
			checkPassed: "Comprobación superada",
			checkFailed: "Comprobación fallida: {{message}}",
		},
		status: {
			saved: "Configuración guardada.",
			savedRestart: "Configuración guardada. Reinicio necesario.",
			restartScheduled:
				"Reinicio programado. El panel puede desconectarse brevemente.",
			restartFailed: "Reinicio fallido: {{message}}",
		},
		restartDialog: {
			title: "¿Programar reinicio del servicio?",
			description:
				"La solicitud solo programa el comando del gestor de servicios. Esta página puede desconectarse antes de que el servicio vuelva a estar disponible.",
			unsavedWarning:
				"Las ediciones no guardadas solo están en esta pestaña del navegador. El reinicio utiliza el archivo persistente y puede hacer que este borrador sea irrecuperable.",
			cancel: "Cancelar",
			scheduleRestart: "Programar reinicio",
		},
		saveReview: {
			title: "Revisar cambios de configuración",
			description:
				"Compruebe el TOML exacto que reemplazará al archivo actual y confirme una segunda vez para guardar.",
			generating: "Generando diff TOML y comprobando el borrador…",
			conflict: "La configuración cambió en el disco",
			previewFailed: "Vista previa fallida",
			reload: "Recargar desde disco y descartar borrador",
			checkPassed: "Comprobación superada",
			checkFailed: "Comprobación fallida",
			securityWarning:
				"Este diff no está redactado intencionadamente. Las contraseñas, tokens, URL de webhook y otras credenciales son visibles en este diálogo autenticado y en la respuesta de red.",
			operationalWarnings: "Advertencias operativas",
			tomlDiff: "Diff TOML",
			noChanges: "No hay cambios en el archivo que guardar.",
			saveFailed: "Error al guardar: {{error}}",
			noRuntimeChange: "Sin cambio en tiempo de ejecución",
			restartRequired: "Reinicio necesario",
			cancel: "Cancelar",
			saveConfig: "Guardar configuración",
			saveAndRestart: "Guardar y programar reinicio",
		},
		warnings: {
			passwordChange:
				"Todas las sesiones se cerrarán tras programar Guardar + Reiniciar.",
			apiDisable: "El panel no estará disponible tras el reinicio.",
			apiEndpointChange: "La dirección del panel puede cambiar tras el reinicio.",
			databasePathChange:
				"El servicio utilizará una base de datos de mensajes diferente tras el reinicio.",
		},
		leaveDialog: {
			title: "¿Salir con cambios sin guardar?",
			description:
				"El borrador de configuración contiene credenciales y no se almacena en el navegador intencionadamente. Salir lo descartará.",
			stay: "Permanecer",
			discard: "Descartar y salir",
		},
		sections: {
			device: "Dispositivo",
			deviceDescription: "Identidad del módem y ruta del objeto",
			sms: "SMS",
			smsDescription: "Filtros de almacenamiento y palabras clave de códigos",
			forwarding: "Reenvío",
			forwardingDescription: "Trabajadores de entrega y perfiles de canal",
			api: "Web API",
			apiDescription: "Acceso al panel y persistencia",
			timeouts: "Tiempos de espera",
			timeoutsDescription: "Límites de ejecución de HTTP y shell",
			retention: "Retención",
			retentionDescription: "Limpieza automática de mensajes",
		},
		fields: {
			device: {
				sectionTitle: "Dispositivo",
				sectionDescription:
					"Identifique este repetidor y seleccione el objeto de ModemManager que recibe y envía mensajes.",
				deviceName: "Nombre del dispositivo",
				deviceNameDescription:
					"Se incluye en las cargas de reenvío para que los canales posteriores puedan identificar el origen.",
				modemPath: "Ruta del objeto módem",
				modemPathDescription:
					"Debe ser una ruta de ModemManager bajo /org/freedesktop/ModemManager1/Modem/.",
			},
			sms: {
				sectionTitle: "SMS",
				sectionDescription:
					"Controle qué ubicaciones de almacenamiento del módem se ignoran y qué frases identifican los mensajes de verificación.",
				ignoredStorage: "Almacenamiento ignorado",
				ignoredStorageDescription:
					"Identificadores de almacenamiento separados por comas, como sm.",
				codeKeywords: "Palabras clave de código",
				codeKeywordsDescription:
					"Frases separadas por comas, sin distinguir mayúsculas de minúsculas, usadas para reconocer códigos de verificación.",
			},
			forwarding: {
				sectionTitle: "Reenvío",
				sectionDescription:
					"Configure la concurrencia de entrega, las credenciales del canal y los perfiles con nombre que reciben mensajes entrantes.",
				concurrency: "Entregas simultáneas",
				concurrencyDescription:
					"Número de trabajos de reenvío procesados a la vez. Rango válido: 1–16.",
			},
			api: {
				sectionTitle: "Web API",
				sectionDescription:
					"Controle la disponibilidad del panel, las direcciones de escucha, la autenticación y la base de datos de mensajes.",
				enableApi: "Habilitar Web API",
				enableApiDescription:
					"Deshabilitar la API eliminará el acceso a este panel tras el reinicio.",
				bindAddress: "Dirección de enlace",
				port: "Puerto",
				portDescription: "Rango válido: 1–65535.",
				ipv6: "Dirección IPv6 complementaria",
				ipv6Description:
					"Escuchar también en una dirección IPv6 complementaria segura cuando se pueda inferir una.",
				password: "Contraseña",
				passwordDescription:
					"Cambiar este valor guarda y programa el reinicio en un solo paso y luego cierra todas las sesiones.",
				databasePath: "Ruta de la base de datos",
			},
			timeouts: {
				sectionTitle: "Tiempos de espera",
				sectionDescription:
					"Limite el establecimiento de la conexión, las solicitudes al proveedor y la ejecución del perfil shell. Todos los valores son en segundos.",
				connectTimeout: "Tiempo de espera de conexión",
				connectTimeoutDescription:
					"Debe ser positivo y no mayor que el tiempo de espera de la solicitud.",
				requestTimeout: "Tiempo de espera de la solicitud",
				shellTimeout: "Tiempo de espera de shell",
			},
			retention: {
				sectionTitle: "Retención",
				sectionDescription:
					"Elimine los mensajes terminales antiguos en lotes limitados conservando los mensajes con entregas activas.",
				enableCleanup: "Habilitar limpieza",
				maxAge: "Antigüedad máxima",
				maxAgeDescription:
					"Los mensajes con más días de antigüedad que este valor serán elegibles para la limpieza.",
				batchSize: "Tamaño del lote",
				batchSizeDescription: "Filas máximas eliminadas en una pasada de limpieza.",
			},
		},
		channel: {
			deliveryRoutes: "Rutas de entrega",
			deliveryRoutesDescription:
				"Habilite los perfiles que deben recibir los mensajes reenviados.",
			profilesActive: "{{enabled}} / {{total}} activos",
			missingProfiles: "Perfiles de reenvío faltantes",
			missingProfilesDescription:
				"Estas referencias habilitadas no coinciden con ningún perfil configurado. Elimínelas para que esta configuración sea válida.",
			removeReference: "Eliminar referencia",
			removeReferenceAria: "Eliminar la referencia de reenvío faltante {{ref}}",
			noProfiles: "Sin perfiles",
			enabled: "Habilitado",
			disabled: "Deshabilitado",
			remove: "Eliminar",
			add: "Añadir",
			profileName: "Nombre del perfil",
			addProfile: "Añadir perfil de {{channel}}",
			duplicateName: "Ese nombre de perfil ya existe.",
			enableAria: "Habilitar reenvío para {{ref}}",
			removeAria: "Eliminar {{ref}}",
			removeDialog: {
				title: "¿Eliminar perfil de reenvío?",
				description:
					"Esto elimina las credenciales del perfil y su referencia habilitada del borrador actual. El cambio no se escribirá hasta que guarde.",
				cancel: "Cancelar",
				remove: "Eliminar perfil",
			},
		},
	},
	login: {
		title: "SmsRelayed",
		password: "Contraseña",
		login: "Iniciar sesión",
		loginFailed: "Inicio de sesión fallido",
		notice: {
			configSavedRestart:
				"Configuración guardada y reinicio programado. Inicie sesión con la nueva contraseña cuando el servicio vuelva.",
		},
	},
	phoneCopy: {
		copy: "Copiar",
		copied: "Copiado",
		copyFailed: "Error al copiar",
		ariaLabel: "Copiar número de teléfono",
		srCopied: "Número de teléfono copiado",
		srFailed: "Error al copiar el número de teléfono",
	},
	language: {
		label: "Idioma",
		en: "English",
		zhCN: "简体中文",
		ja: "日本語",
		ko: "한국어",
		fr: "Français",
		es: "Español",
	},
} satisfies TranslationShape<typeof en>;
