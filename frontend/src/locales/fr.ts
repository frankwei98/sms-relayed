import type { en } from "./en";

type TranslationShape<T> = {
	[K in keyof T]: T[K] extends string ? string : TranslationShape<T[K]>;
};

export const fr = {
	nav: {
		sms: "SMS",
		modem: "Modem",
		forwarding: "Transfert",
		config: "Configuration",
	},
	header: {
		ariaPrimary: "Principal",
		ariaBackConfig: "Retour à la configuration",
		ariaConfigCategories: "Catégories de configuration",
		ariaUnsavedChanges: "Modifications non enregistrées",
		logout: "Déconnexion",
	},
	common: {
		refresh: "Actualiser",
		cancel: "Annuler",
		save: "Enregistrer",
		check: "Vérifier",
		restart: "Redémarrer",
		retry: "Réessayer",
		done: "Terminé",
		search: "Rechercher",
		add: "Ajouter",
		remove: "Supprimer",
	},
	modem: {
		title: "Modem",
		lastChecked: "Dernière vérification {{time}}",
		statusUnavailable: "Statut indisponible",
		loading: "Chargement du statut du modem…",
		refresh: "Actualiser",
		field: {
			configuredPath: "Chemin configuré",
			resolvedModem: "Modem résolu",
			enabled: "Activé",
			state: "État",
			sim: "SIM",
			phoneNumber: "Numéro de téléphone",
			operator: "Opérateur",
			signal: "Signal",
			access: "Accès",
			messaging: "Messagerie",
			mmcli: "mmcli",
		},
		value: {
			yes: "Oui",
			no: "Non",
			unknown: "Inconnu",
			notFound: "Introuvable",
			notReported: "Non signalé",
			available: "Disponible",
			unavailable: "Indisponible",
			missing: "Absent",
			notSelected: "Non sélectionné",
		},
		status: {
			ok: "OK",
			degraded: "DÉGRADÉ",
			error: "ERREUR",
			unknown: "INCONNU",
		},
		smsOverIms: {
			title: "SMS over IMS",
			description:
				"Signalé par le modem ; cela ne prouve pas la route utilisée par chaque message.",
			field: {
				configured: "Configuré",
				registration: "Enregistrement",
				smsService: "Service SMS",
				technology: "Technologie",
				qmicli: "qmicli",
				qmiDevice: "Périphérique QMI",
				evidence: "Preuve",
			},
			enum: {
				enabled: "Activé",
				disabled: "Désactivé",
				registered: "Enregistré",
				registering: "Enregistrement en cours",
				limited: "Limité",
				notRegistered: "Non enregistré",
				notAvailable: "Non disponible",
				available: "Disponible",
				unknown: "Inconnu",
			},
			availableOverWlan: "Disponible via WLAN",
			technology: {
				wwan: "WWAN",
				wlan: "WLAN",
				interworkingWlan: "Interworking WLAN",
				unknown: "Inconnu",
			},
			evidence: {
				qmiImsa: "QMI IMSA",
				qmiIms: "QMI IMS",
				noEvidence: "Aucune preuve d'exécution",
			},
		},
		actions: {
			enable: "Activer",
			disable: "Désactiver",
		},
		dangerZone: {
			title: "Zone dangereuse",
			reset: "Réinitialiser le modem",
			dialogTitle: "Réinitialiser le modem ?",
			dialogDescription:
				"Cela peut déconnecter le service cellulaire et faire disparaître le modem le temps de sa réénumération.",
			cancel: "Annuler",
			confirmReset: "Réinitialiser",
		},
		diagnostics: {
			title: "Diagnostics",
			reasons: "Raisons : {{reasons}}",
			possibleNewPath: "Nouveau chemin modem possible : {{path}}",
			error: "Erreur : {{error}}",
		},
		imsDiagnostics: {
			imsProbeNotAttempted: "La sonde IMS n'a pas été tentée.",
			modemNotResolved:
				"La sonde IMS a été ignorée car aucun modem n'a été résolu.",
			modemDisabled: "La sonde IMS a été ignorée car le modem est désactivé.",
			qmicliMissing: "qmicli n'est pas installé ou n'a pas pu être exécuté.",
			qmicliPathInvalid: "Le chemin qmicli configuré est invalide.",
			qmicliProbeFailed: "La détection de capacité qmicli a échoué.",
			imsProbePermissionDenied:
				"qmicli n'a pas pu être exécuté en raison des permissions.",
			qmiPortUnavailable:
				"Aucun port de contrôle QMI n'a été signalé par ModemManager.",
			qmiPortAmbiguous: "Plus d'un port de contrôle QMI a été signalé.",
			qmiProxyUnavailable: "Le proxy QMI est indisponible.",
			imsProbeTimeout: "La sonde IMS a dépassé son budget de temps.",
			imsServicesQueryFailed: "La requête de services IMS a échoué.",
			imsServicesQueryUnavailable:
				"qmicli n'expose pas la requête de services IMS.",
			imsRegistrationQueryFailed: "La requête d'enregistrement IMS a échoué.",
			imsRegistrationQueryUnavailable:
				"qmicli n'expose pas la requête d'enregistrement IMS.",
			imsSettingsQueryFailed: "La requête de paramètres IMS a échoué.",
			imsSettingsQueryUnavailable:
				"qmicli n'expose pas la requête de paramètres IMS.",
			imsServicesOutputUnrecognized:
				"La réponse du service IMS n'a pas été reconnue.",
			imsRegistrationOutputUnrecognized:
				"La réponse d'enregistrement IMS n'a pas été reconnue.",
			imsSettingsOutputUnrecognized:
				"La réponse des paramètres IMS n'a pas été reconnue.",
			imsServicesOutputNonstandard:
				"La réponse du service IMS utilisait un libellé non standard.",
			imsRegistrationOutputNonstandard:
				"La réponse d'enregistrement IMS utilisait un libellé non standard.",
			imsSettingsOutputNonstandard:
				"La réponse des paramètres IMS utilisait un libellé non standard.",
			imsStateInconsistent:
				"Le modem a signalé une configuration IMS et un état d'exécution incohérents.",
			fallback:
				"Les informations de diagnostic IMS supplémentaires sont indisponibles.",
		},
	},
	forwarding: {
		sidebar: {
			title: "Transfert",
			operations: "Opérations",
			ariaLabel: "Profils de transfert",
			ariaViews: "Vues de transfert",
			ariaRefresh: "Actualiser le statut de transfert",
			ariaClose: "Fermer la navigation de transfert",
			ariaOpen: "Ouvrir la navigation de transfert",
		},
		overview: {
			title: "Aperçu",
			subtitle: "Instantanés de tous les profils",
			configured: "Configuré",
			historical: "Historique",
			noConfiguredProfiles: "Aucun profil configuré",
			noHistoricalProfiles: "Aucun profil historique conservé",
		},
		snapshot: {
			generated: "Instantané généré",
			generatedDescription:
				"Jusqu'à {{limit}} tentatives conservées par profil",
		},
		loading: "Chargement du statut de transfert…",
		error: {
			title: "Impossible de charger le statut de transfert",
			description: "L'instantané de transfert n'a pas pu être chargé.",
			refreshFailed: "Échec de l'actualisation",
			refreshDescription: "Affichage de l'instantané précédent. {{error}}",
		},
		srLive: {
			refreshing: "Actualisation du statut de transfert.",
			snapshot: "Instantané de transfert généré {{time}}.",
		},
		detail: {
			overview: "Aperçu",
			overviewSubtitle:
				"Profils configurés et historique des tentatives conservées",
			retainedAttempts: "Tentatives de transfert conservées",
			notPresent: "Absent du dernier instantané",
			lastUpdated: "Dernière mise à jour {{time}}",
		},
		overviewSection: {
			currentSnapshot: "Instantané actuel",
			coverage: "Couverture de transfert",
			description:
				"État de la configuration et disponibilité des tentatives conservées du dernier instantané backend.",
			configuredProfiles: "Profils configurés",
			enabledProfiles: "Profils activés",
			profilesWithAttempts: "Profils avec tentatives conservées",
			profileSnapshot: "Instantané des profils",
			profileSnapshotDescription:
				"Dernier résultat conservé pour chaque profil configuré ou historique.",
			empty: "Aucun profil de transfert configuré.",
			emptyDescription:
				"Aucune tentative de profil historique conservée n'est disponible non plus.",
		},
		table: {
			ariaLabel: "Instantané des profils de transfert",
			profile: "Profil",
			state: "État",
			latestOutcome: "Dernier résultat",
			latestCompleted: "Dernier terminé",
			retained: "Conservé",
			attempt: "Tentative",
			completed: "Terminé",
			outcome: "Résultat",
			timing: "Durée",
			error: "Erreur",
		},
		profile: {
			unavailable: "Profil indisponible",
			unavailableDescription:
				"Le profil {{key}} n'est pas présent dans le dernier instantané de transfert.",
			viewOverview: "Voir l'aperçu",
		},
		badge: {
			configured: "Configuré",
			enabled: "Activé",
			disabled: "Désactivé",
			historical: "Historique",
			retry: "Réessayer",
		},
		outcome: {
			success: "Succès",
			transientFailure: "Échec temporaire",
			permanentFailure: "Échec permanent",
			unknown: "Résultat inconnu",
			noAttempts: "Aucune tentative",
			latest: "Dernier : {{outcome}}",
		},
		attempts: {
			title_one: "{{count}} dernière tentative",
			title_other: "{{count}} dernières tentatives",
			retainedDescription: "{{count}} conservées dans cet instantané",
			newestFirst: "Plus récent en premier",
			empty: "Aucune tentative de transfert pour le moment.",
			emptyDescription:
				"Cet instantané ne contient aucune tentative conservée pour ce profil.",
			label: "{{label}} pour {{key}}",
		},
		mobile: {
			completed: "Terminé",
			timing: "Durée",
			error: "Erreur",
			retained: "{{count}} conservées",
		},
		timing: {
			dispatch: "Expédition {{time}}",
			request: "Requête {{time}}",
		},
	},
	messages: {
		title: "Messages",
		sim: "SIM {{number}}",
		aria: {
			newMessage: "Nouveau message",
			filters: "Filtres",
			backConversations: "Retour aux conversations",
			markConversationRead: "Marquer la conversation comme lue",
			messageTimeline: "Chronologie des messages",
			conversationActions: "Actions de conversation",
			sendMessage: "Envoyer le message",
			searchMessages: "Rechercher des messages",
		},
		search: {
			placeholder: "Rechercher des messages",
		},
		filter: {
			title: "Outils de message",
			description:
				"Filtrer la boîte de réception ou exporter la vue de messages actuelle.",
			direction: "Direction",
			allDirections: "Toutes les directions",
			inbound: "Réception",
			outbound: "Envoi",
			status: "Statut",
			allStatuses: "Tous les statuts",
			received: "Reçu",
			sending: "Envoi en cours",
			sent: "Envoyé",
			failed: "Échoué",
			unreadOnly: "Non lus uniquement",
			exportCsv: "CSV",
			exportJson: "JSON",
			done: "Terminé",
			search: "Rechercher",
		},
		conversationList: {
			empty: "Aucune conversation",
			emptyDescription: "Les fils SMS entrants et sortants apparaîtront ici.",
			messages: "{{count}} messages",
			noMatching: "Aucun message correspondant",
			noMatchingDescription:
				"Ajustez les filtres ou attendez le prochain événement SMS.",
		},
		error: {
			load: "Impossible de charger les messages.",
			markRead: "Impossible de marquer les messages comme lus. Réessayez.",
			update: "Impossible de mettre à jour les messages. Réessayez.",
			delete: "Impossible de supprimer les messages. Réessayez.",
			export: "Impossible d’exporter les messages. Réessayez.",
			loadOlder: "Impossible de charger les messages précédents. Réessayez.",
			refresh: "Impossible d’actualiser les messages. Réessayez.",
		},
		thread: {
			loadingOlder: "Chargement des messages plus anciens",
			loadOlder: "Charger les messages plus anciens",
			newMessage: "Nouveau message",
			newMessageSubtitle: "Choisissez un destinataire et rédigez un SMS",
			selectConversation: "Sélectionner une conversation",
			selectConversationSubtitle: "Choisissez un fil dans la liste",
			noThreadSelected: "Aucun fil sélectionné",
			noThreadDescription:
				"Choisissez une conversation ou commencez un nouveau SMS.",
			recipientLabel: "À",
			recipientPlaceholder: "Numéro de téléphone",
			composerPlaceholder: "Message",
			sendMessage: "Envoyer",
			sendingMessage: "Envoi…",
			sendFailed: "Le message n’a pas pu être envoyé. Réessayez.",
		},
		direction: {
			sent: "Envoyé",
			inbox: "Boîte de réception",
			failed: "Échoué",
		},
		actions: {
			selectMessages: "Sélectionner des messages",
			stopSelecting: "Arrêter la sélection",
			markRead: "Marquer comme lu ({{count}})",
			markUnread: "Marquer comme non lu ({{count}})",
			deleteSelected: "Supprimer la sélection",
			markConversationRead: "Marquer la conversation comme lue",
			conversationActions: "Actions de conversation",
		},
		relativeDay: {
			today: "Aujourd'hui",
			yesterday: "Hier",
			daysAgo: "Il y a {{count}} jours",
		},
	},
	config: {
		sidebar: {
			title: "Configuration",
			ariaLabel: "Catégories de configuration",
			ariaUnsaved: "Modifications non enregistrées",
			categories: "Catégories",
			dirty: "{{count}} {{category}} modifiée(s)",
			dirty_one: "{{count}} catégorie modifiée",
			dirty_other: "{{count}} catégories modifiées",
			clean: "Aucune modification non enregistrée",
		},
		editor: {
			unsavedDraft: "Brouillon non enregistré",
			saved: "Configuration enregistrée",
			restartRequired: "Redémarrage requis",
			loading: "Chargement de la configuration…",
		},
		error: {
			title: "Configuration indisponible",
		},
		action: {
			save: "Enregistrer",
			check: "Vérifier",
			restart: "Redémarrer",
			checking: "Vérification du brouillon complet…",
			notChecked: "Non vérifié",
			checkPassed: "Vérification réussie",
			checkFailed: "Échec de la vérification : {{message}}",
		},
		status: {
			saved: "Configuration enregistrée.",
			savedRestart: "Configuration enregistrée. Redémarrage requis.",
			restartScheduled:
				"Redémarrage planifié. Le tableau de bord peut se déconnecter brièvement.",
			restartFailed: "Échec du redémarrage : {{message}}",
		},
		restartDialog: {
			title: "Planifier le redémarrage du service ?",
			description:
				"La requête planifie uniquement la commande service-manager. Cette page peut se déconnecter avant que le service ne soit à nouveau disponible.",
			unsavedWarning:
				"Les modifications non enregistrées ne sont que dans cet onglet du navigateur. Le redémarrage utilise le fichier persistant et peut rendre ce brouillon irrécupérable.",
			cancel: "Annuler",
			scheduleRestart: "Planifier le redémarrage",
		},
		saveReview: {
			title: "Examiner les modifications de configuration",
			description:
				"Vérifiez le TOML exact qui remplacera le fichier actuel, puis confirmez une seconde fois pour enregistrer.",
			generating: "Génération du diff TOML et vérification du brouillon…",
			conflict: "La configuration a changé sur le disque",
			previewFailed: "Échec de l'aperçu",
			reload: "Recharger depuis le disque et abandonner le brouillon",
			checkPassed: "Vérification réussie",
			checkFailed: "Échec de la vérification",
			securityWarning:
				"Ce diff est intentionnellement non masqué. Les mots de passe, jetons, URL de webhook et autres identifiants sont visibles dans cette boîte de dialogue authentifiée et dans la réponse réseau.",
			operationalWarnings: "Avertissements opérationnels",
			tomlDiff: "Diff TOML",
			noChanges: "Aucun changement de fichier à enregistrer.",
			saveFailed: "Échec de l'enregistrement : {{error}}",
			noRuntimeChange: "Aucun changement d'exécution",
			restartRequired: "Redémarrage requis",
			cancel: "Annuler",
			saveConfig: "Enregistrer la configuration",
			saveAndRestart: "Enregistrer et planifier le redémarrage",
		},
		warnings: {
			passwordChange:
				"Toutes les sessions seront déconnectées après l'enregistrement et la planification du redémarrage.",
			apiDisable: "Le tableau de bord sera indisponible après le redémarrage.",
			apiEndpointChange:
				"L'adresse du tableau de bord peut changer après le redémarrage.",
			databasePathChange:
				"Le service utilisera une base de données de messages différente après le redémarrage.",
		},
		leaveDialog: {
			title: "Quitter avec des modifications non enregistrées ?",
			description:
				"Le brouillon de configuration contient des identifiants et n'est intentionnellement pas stocké dans le navigateur. Quitter l'abandonnera.",
			stay: "Rester",
			discard: "Abandonner et quitter",
		},
		sections: {
			device: "Appareil",
			deviceDescription: "Identité du modem et chemin d'objet",
			sms: "SMS",
			smsDescription: "Filtres de stockage et mots-clés de code",
			forwarding: "Transfert",
			forwardingDescription: "Workers de livraison et profils de canal",
			api: "Web API",
			apiDescription: "Accès au tableau de bord et persistance",
			timeouts: "Délais d'attente",
			timeoutsDescription: "Limites d'exécution HTTP et shell",
			retention: "Rétention",
			retentionDescription: "Nettoyage automatique des messages",
		},
		fields: {
			device: {
				sectionTitle: "Appareil",
				sectionDescription:
					"Identifiez ce relais et sélectionnez l'objet ModemManager qui reçoit et envoie les messages.",
				deviceName: "Nom de l'appareil",
				deviceNameDescription:
					"Inclus dans les charges de transfert afin que les canaux en aval puissent identifier la source.",
				modemPath: "Chemin d'objet du modem",
				modemPathDescription:
					"Doit être un chemin ModemManager sous /org/freedesktop/ModemManager1/Modem/.",
			},
			sms: {
				sectionTitle: "SMS",
				sectionDescription:
					"Contrôlez quels emplacements de stockage du modem sont ignorés et quelles phrases identifient les messages de code de vérification.",
				ignoredStorage: "Stockage ignoré",
				ignoredStorageDescription:
					"Identifiants de stockage séparés par des virgules, tels que sm.",
				codeKeywords: "Mots-clés de code",
				codeKeywordsDescription:
					"Phrases séparées par des virgules, insensibles à la casse, utilisées pour reconnaître les codes de vérification.",
			},
			forwarding: {
				sectionTitle: "Transfert",
				sectionDescription:
					"Configurez la concurrence de livraison, les identifiants de canal et les profils nommés qui reçoivent les messages entrants.",
				concurrency: "Livraisons simultanées",
				concurrencyDescription:
					"Nombre de tâches de transfert traitées en même temps. Plage valide : 1–16.",
			},
			api: {
				sectionTitle: "Web API",
				sectionDescription:
					"Contrôlez la disponibilité du tableau de bord, les adresses d'écoute, l'authentification et la base de données de messages.",
				enableApi: "Activer la Web API",
				enableApiDescription:
					"Désactiver l'API supprime l'accès à ce tableau de bord après le redémarrage.",
				bindAddress: "Adresse de liaison",
				port: "Port",
				portDescription: "Plage valide : 1–65535.",
				ipv6: "Compagnon IPv6",
				ipv6Description:
					"Écoute également sur une adresse compagnon IPv6 sûre lorsqu'elle peut être déduite.",
				password: "Mot de passe",
				passwordDescription:
					"Modifier cette valeur enregistre et planifie le redémarrage en une seule étape, puis déconnecte toutes les sessions.",
				databasePath: "Chemin de la base de données",
			},
			timeouts: {
				sectionTitle: "Délais d'attente",
				sectionDescription:
					"Limitez l'établissement de la connexion, les requêtes au fournisseur et l'exécution du profil shell. Toutes les valeurs sont en secondes.",
				connectTimeout: "Délai de connexion",
				connectTimeoutDescription:
					"Doit être positif et inférieur ou égal au délai de requête.",
				requestTimeout: "Délai de requête",
				shellTimeout: "Délai shell",
			},
			retention: {
				sectionTitle: "Rétention",
				sectionDescription:
					"Supprimez les anciens messages terminaux par lots limités tout en conservant les messages avec des livraisons actives.",
				enableCleanup: "Activer le nettoyage",
				maxAge: "Âge maximal",
				maxAgeDescription:
					"Les messages plus anciens que ce nombre de jours deviennent éligibles au nettoyage.",
				batchSize: "Taille du lot",
				batchSizeDescription:
					"Nombre maximal de lignes supprimées par passage de nettoyage.",
			},
		},
		channel: {
			deliveryRoutes: "Itinéraires de livraison",
			deliveryRoutesDescription:
				"Activez les profils qui doivent recevoir les messages transférés.",
			profilesActive: "{{enabled}} / {{total}} actifs",
			missingProfiles: "Profils de transfert manquants",
			missingProfilesDescription:
				"Ces références activées ne correspondent à aucun profil configuré. Supprimez-les pour rendre cette configuration valide.",
			removeReference: "Supprimer la référence",
			removeReferenceAria:
				"Supprimer la référence de transfert manquante {{ref}}",
			noProfiles: "Aucun profil",
			enabled: "Activé",
			disabled: "Désactivé",
			remove: "Supprimer",
			add: "Ajouter",
			profileName: "Nom du profil",
			addProfile: "Ajouter un profil {{channel}}",
			duplicateName: "Ce nom de profil existe déjà.",
			enableAria: "Activer le transfert pour {{ref}}",
			removeAria: "Supprimer {{ref}}",
			removeDialog: {
				title: "Supprimer le profil de transfert ?",
				description:
					"Cela supprime les identifiants du profil et sa référence activée du brouillon actuel. La modification n'est pas écrite tant que vous n'enregistrez pas.",
				cancel: "Annuler",
				remove: "Supprimer le profil",
			},
		},
	},
	login: {
		title: "SmsRelayed",
		password: "Mot de passe",
		login: "Connexion",
		loginFailed: "Échec de la connexion",
		notice: {
			configSavedRestart:
				"Configuration enregistrée et redémarrage planifié. Connectez-vous avec le nouveau mot de passe une fois le service rétabli.",
		},
	},
	phoneCopy: {
		copy: "Copier",
		copied: "Copié",
		copyFailed: "Échec de la copie",
		ariaLabel: "Copier le numéro de téléphone",
		srCopied: "Numéro de téléphone copié",
		srFailed: "Échec de la copie du numéro de téléphone",
	},
	language: {
		label: "Langue",
		en: "English",
		zhCN: "简体中文",
		ja: "日本語",
		ko: "한국어",
		fr: "Français",
		es: "Español",
	},
} satisfies TranslationShape<typeof en>;
