import connectors from "../en/connectors";

export default {
	...connectors,
	connections: {
		...connectors.connections,
		addGmail: "Ajouter une connexion Gmail",
		addHimalaya: "Ajouter une connexion Himalaya",
		addGmailTitle: "Connexion Gmail",
		addHimalayaTitle: "Connexion de messagerie Himalaya",
		emailNamePlaceholder: "Messagerie professionnelle",
		himalayaAccount: "Nom du compte Himalaya",
		himalayaBackend: "Moteur de stockage",
		gmailCredentialHelp:
			"Utilise l’autorisation Google Workspace existante dans Moltis. Les identifiants Gmail sont référencés et ne sont pas copiés dans la base de données des connecteurs.",
		emailRequired: "Saisissez un nom de connexion et, le cas échéant, le nom du compte Himalaya configuré.",
		googleWorkspaceCredentials: "Identifiants Google Workspace",
		testEmailTitle: "État de préparation de la connexion de messagerie",
		emailReady: "Le compte de messagerie est prêt pour la synchronisation.",
		emailReadyAddress: "Gmail est prêt pour {{email}}.",
	},
	datasets: {
		...connectors.datasets,
		addEmailTitle: "Ajouter un jeu de données de messagerie",
		editEmailTitle: "Modifier le jeu de données de messagerie",
		emailHelp:
			"La synchronisation des e-mails est en lecture seule et limitée. Le contenu des messages est stocké localement en tant que données externes non fiables ; les pièces jointes ne sont représentées que par leurs métadonnées.",
		mailboxes: "Boîtes aux lettres",
		mailboxesHelp: "Un identifiant de boîte aux lettres Himalaya par ligne, 32 au maximum.",
		emailQuery: "Requête de recherche",
		emailQueryHelp:
			"Recherche facultative auprès du fournisseur. Laissez ce champ vide pour récupérer les messages les plus récents dans la portée sélectionnée.",
		emailLimit: "Limite de messages",
		includeBodies: "Stocker le corps des messages avec une taille limitée",
		emailRequired: "Choisissez une connexion de messagerie et saisissez un nom de jeu de données.",
		emailLimitInvalid: "La limite de messages doit être un nombre entier compris entre 1 et 1 000.",
		mailboxesInvalid: "Saisissez entre 1 et 32 identifiants de boîtes aux lettres séparés par des virgules.",
		gmailScope: "Requête Gmail {{query}} · jusqu’à {{limit}} messages",
		himalayaScope: "Boîtes aux lettres {{mailboxes}} · jusqu’à {{limit}} messages",
		allMail: "tous les e-mails",
	},
};
