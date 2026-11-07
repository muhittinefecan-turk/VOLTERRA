//
// LocalizationManager.swift
// VolterraMac
//
// 18-Language Real-Time Localization Engine with RTL Support & UserDefaults Persistence
//

import Foundation
import SwiftUI

public enum AppLanguage: String, CaseIterable, Identifiable, Codable, Sendable {
    case turkish = "tr"
    case english = "en"
    case german = "de"
    case french = "fr"
    case spanish = "es"
    case italian = "it"
    case portuguese = "pt"
    case russian = "ru"
    case japanese = "ja"
    case korean = "ko"
    case chineseSimplified = "zh-Hans"
    case chineseTraditional = "zh-Hant"
    case arabic = "ar"
    case hindi = "hi"
    case dutch = "nl"
    case polish = "pl"
    case swedish = "sv"
    case ukrainian = "uk"

    public var id: String { rawValue }

    public var flag: String {
        switch self {
        case .turkish: return "🇹🇷"
        case .english: return "🇺🇸"
        case .german: return "🇩🇪"
        case .french: return "🇫🇷"
        case .spanish: return "🇪🇸"
        case .italian: return "🇮🇹"
        case .portuguese: return "🇧🇷"
        case .russian: return "🇷🇺"
        case .japanese: return "🇯🇵"
        case .korean: return "🇰🇷"
        case .chineseSimplified: return "🇨🇳"
        case .chineseTraditional: return "🇹🇼"
        case .arabic: return "🇸🇦"
        case .hindi: return "🇮🇳"
        case .dutch: return "🇳🇱"
        case .polish: return "🇵🇱"
        case .swedish: return "🇸🇪"
        case .ukrainian: return "🇺🇦"
        }
    }

    public var nativeName: String {
        switch self {
        case .turkish: return "Türkçe"
        case .english: return "English"
        case .german: return "Deutsch"
        case .french: return "Français"
        case .spanish: return "Español"
        case .italian: return "Italiano"
        case .portuguese: return "Português"
        case .russian: return "Русский"
        case .japanese: return "日本語"
        case .korean: return "한국어"
        case .chineseSimplified: return "简体中文"
        case .chineseTraditional: return "繁體中文"
        case .arabic: return "العربية"
        case .hindi: return "हिन्दी"
        case .dutch: return "Nederlands"
        case .polish: return "Polski"
        case .swedish: return "Svenska"
        case .ukrainian: return "Українська"
        }
    }

    public var displayLabel: String {
        "\(flag) \(nativeName)"
    }

    public var isRightToLeft: Bool {
        self == .arabic
    }

    public var layoutDirection: LayoutDirection {
        isRightToLeft ? .rightToLeft : .leftToRight
    }
}

public enum L10nKey: String, CaseIterable, Sendable {
    // Sidebar & Navigation
    case sectionOperations
    case sectionDiagnostics
    case sectionAudit
    case tabSetup
    case tabLiveTransfer
    case tabPerformance
    case tabErrorCenter
    case tabHistory
    case refreshDisksHelp
    case languageMenuTitle

    // DashboardView
    case setupTitle
    case setupSubtitle
    case sourceLocation
    case destinationLocation
    case advancedOptions
    case directIOToggle
    case readbackVerifyToggle
    case memoryBufferBudget
    case startTransfer

    // LiveTransferView
    case activeJob
    case noActiveTransfer
    case currentSpeed
    case estimatedTime
    case activeChunkEngine
    case readWriteLatency
    case bufferUtilization
    case quarantinedFiles
    case throughputLast60s
    case pauseTransfer
    case resumeTransfer
    case cancelTransfer

    // PerformanceDashboardView
    case perfTitle
    case perfSubtitle
    case sourceDeviceRead
    case destDeviceWrite
    case throughputProfiles
    case adaptiveChunkLadder

    // ErrorCenterView
    case errorCenterTitle
    case errorCenterSubtitle
    case skipAllQuarantined
    case retryAll
    case searchFilePath
    case noQuarantinedFiles
    case healthyPipelineSubtitle
    case inspectAction
    case skipAction
    case retryAction

    // HistoryView
    case historyTitle
    case historySubtitle
    case refreshButton
    case searchJobOrPath
    case maskPersonalPaths
    case noHistoryFound
}

@MainActor
public final class LocalizationManager: ObservableObject {
    public static let shared = LocalizationManager()

    private static let storageKey = "volterra.selectedLanguage"

    @Published public var currentLanguage: AppLanguage {
        didSet {
            UserDefaults.standard.set(currentLanguage.rawValue, forKey: Self.storageKey)
        }
    }

    public init(initialLanguage: AppLanguage? = nil) {
        if let explicit = initialLanguage {
            self.currentLanguage = explicit
        } else if let savedRaw = UserDefaults.standard.string(forKey: Self.storageKey),
                  let saved = AppLanguage(rawValue: savedRaw) {
            self.currentLanguage = saved
        } else {
            self.currentLanguage = Self.detectSystemLanguage()
        }
    }

    public static func detectSystemLanguage() -> AppLanguage {
        guard let preferred = Locale.preferredLanguages.first?.lowercased() else {
            return .turkish
        }
        if preferred.hasPrefix("tr") { return .turkish }
        if preferred.hasPrefix("en") { return .english }
        if preferred.hasPrefix("de") { return .german }
        if preferred.hasPrefix("fr") { return .french }
        if preferred.hasPrefix("es") { return .spanish }
        if preferred.hasPrefix("it") { return .italian }
        if preferred.hasPrefix("pt") { return .portuguese }
        if preferred.hasPrefix("ru") { return .russian }
        if preferred.hasPrefix("ja") { return .japanese }
        if preferred.hasPrefix("ko") { return .korean }
        if preferred.hasPrefix("zh-hant") || preferred.hasPrefix("zh-tw") || preferred.hasPrefix("zh-hk") {
            return .chineseTraditional
        }
        if preferred.hasPrefix("zh") { return .chineseSimplified }
        if preferred.hasPrefix("ar") { return .arabic }
        if preferred.hasPrefix("hi") { return .hindi }
        if preferred.hasPrefix("nl") { return .dutch }
        if preferred.hasPrefix("pl") { return .polish }
        if preferred.hasPrefix("sv") { return .swedish }
        if preferred.hasPrefix("uk") { return .ukrainian }
        return .turkish
    }

    public func setLanguage(_ language: AppLanguage) {
        self.currentLanguage = language
    }

    public func t(_ key: L10nKey) -> String {
        if let table = Self.translations[currentLanguage], let val = table[key] {
            return val
        }
        return Self.translations[.english]?[key] ?? key.rawValue
    }

    // Complete 18-Language Dictionary
    public static let translations: [AppLanguage: [L10nKey: String]] = [
        .turkish: [
            .sectionOperations: "İşlemler",
            .sectionDiagnostics: "Tanılama & Analiz",
            .sectionAudit: "Denetim & Kayıtlar",
            .tabSetup: "Kurulum",
            .tabLiveTransfer: "Canlı Aktarım",
            .tabPerformance: "Performans (ADP/ITO)",
            .tabErrorCenter: "Hata Merkezi",
            .tabHistory: "Geçmiş ve Raporlar",
            .refreshDisksHelp: "Bağlı diskleri yenile",
            .languageMenuTitle: "Dil Seçimi (18 Dil)",
            .setupTitle: "Aktarım Kurulumu",
            .setupSubtitle: "Kaynak ve hedef diskleri seçin, aktarım politikasını belirleyin ve kapasiteyi doğrulayın.",
            .sourceLocation: "Kaynak Disk / Klasör",
            .destinationLocation: "Hedef Disk / Klasör",
            .advancedOptions: "Gelişmiş Aktarım Seçenekleri",
            .directIOToggle: "Doğrudan I/O (F_NOCACHE ile işletim sistemi önbelleğini atla)",
            .readbackVerifyToggle: "Fiziksel Geri-Okuma Doğrulaması (BLAKE3 hedef bayt kontrolü)",
            .memoryBufferBudget: "Bellek Tampon Bütçesi:",
            .startTransfer: "Aktarımı Başlat",
            .activeJob: "Aktif Görev:",
            .noActiveTransfer: "Aktif Aktarım Yok",
            .currentSpeed: "Anlık Hız",
            .estimatedTime: "Tahmini Süre (ETA)",
            .activeChunkEngine: "Aktif Parça Motoru",
            .readWriteLatency: "Okuma / Yazma Gecikmesi",
            .bufferUtilization: "Tampon Kullanımı",
            .quarantinedFiles: "Karantinadaki Dosyalar",
            .throughputLast60s: "Aktarım Hızı Grafiği (Son 60 Saniye)",
            .pauseTransfer: "Duraklat",
            .resumeTransfer: "Devam Et",
            .cancelTransfer: "Aktarımı İptal Et",
            .perfTitle: "Performans ve Darboğaz İzleyici",
            .perfSubtitle: "Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO) gerçek zamanlı telemetri.",
            .sourceDeviceRead: "Kaynak Cihaz (Okuma)",
            .destDeviceWrite: "Hedef Cihaz (Yazma)",
            .throughputProfiles: "Hız Profilleri (MB/s)",
            .adaptiveChunkLadder: "Adaptif Parça Boyutu Merdiveni",
            .errorCenterTitle: "Hata Karantina Merkezi",
            .errorCenterSubtitle: "Okunamayan dosyalar aktarımı durdurmadan izole edilir. Dosya atlamak hiçbir veriyi silmez.",
            .skipAllQuarantined: "Tümünü Pas Geç",
            .retryAll: "Tümünü Yeniden Dene",
            .searchFilePath: "Dosya yolu ara...",
            .noQuarantinedFiles: "Karantinada Dosya Yok",
            .healthyPipelineSubtitle: "Aktarım hattı sağlıklı: tüm dosyalar başarıyla işlendi.",
            .inspectAction: "İncele",
            .skipAction: "Pas Geç",
            .retryAction: "Tekrar Dene",
            .historyTitle: "İş Geçmişi ve Denetim Raporları",
            .historySubtitle: "SQLite WAL günlüğünde saklanan geçmiş aktarım kayıtları.",
            .refreshButton: "Yenile",
            .searchJobOrPath: "Görev Kimliği veya Yol Ara...",
            .maskPersonalPaths: "Kişisel Yolları Maskele",
            .noHistoryFound: "Geçmiş Görev Bulunamadı"
        ],
        .english: [
            .sectionOperations: "Operations",
            .sectionDiagnostics: "Diagnostics",
            .sectionAudit: "Audit",
            .tabSetup: "Setup",
            .tabLiveTransfer: "Live Transfer",
            .tabPerformance: "Performance",
            .tabErrorCenter: "Error Center",
            .tabHistory: "History & Reports",
            .refreshDisksHelp: "Refresh connected disk devices",
            .languageMenuTitle: "Language (18 Languages)",
            .setupTitle: "Transfer Setup",
            .setupSubtitle: "Select source and target volumes, choose transfer policy, and verify capacity.",
            .sourceLocation: "Source Location",
            .destinationLocation: "Destination Location",
            .advancedOptions: "Advanced Transfer Options",
            .directIOToggle: "Direct I/O (Bypass OS Cache via F_NOCACHE)",
            .readbackVerifyToggle: "Physical Readback Verification (BLAKE3 verify target bytes)",
            .memoryBufferBudget: "Memory Buffer Budget:",
            .startTransfer: "Start Transfer",
            .activeJob: "Active Job:",
            .noActiveTransfer: "No Active Transfer",
            .currentSpeed: "Current Speed",
            .estimatedTime: "Estimated Time (ETA)",
            .activeChunkEngine: "Active Chunk Engine",
            .readWriteLatency: "Read / Write Latency",
            .bufferUtilization: "Buffer Utilization",
            .quarantinedFiles: "Quarantined Files",
            .throughputLast60s: "Throughput Activity (Last 60 Seconds)",
            .pauseTransfer: "Pause Transfer",
            .resumeTransfer: "Resume Transfer",
            .cancelTransfer: "Cancel Transfer",
            .perfTitle: "Performance & Bottleneck Monitor",
            .perfSubtitle: "Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO) real-time telemetry.",
            .sourceDeviceRead: "Source Device (Read)",
            .destDeviceWrite: "Destination Device (Write)",
            .throughputProfiles: "Throughput Profiles (MB/s)",
            .adaptiveChunkLadder: "Adaptive Chunk Engine Ladder",
            .errorCenterTitle: "Error Quarantine Center",
            .errorCenterSubtitle: "Unreadable files are isolated without stalling the transfer. Skipping a file never deletes any data.",
            .skipAllQuarantined: "Skip All Quarantined",
            .retryAll: "Retry All",
            .searchFilePath: "Search file path...",
            .noQuarantinedFiles: "No Quarantined Files",
            .healthyPipelineSubtitle: "Healthy transfer pipeline: all files processed successfully.",
            .inspectAction: "Inspect",
            .skipAction: "Skip",
            .retryAction: "Retry",
            .historyTitle: "Job History & Audit Reports",
            .historySubtitle: "Durable record of past transfer jobs stored in SQLite WAL journal.",
            .refreshButton: "Refresh",
            .searchJobOrPath: "Search Job ID or Path...",
            .maskPersonalPaths: "Mask Personal Paths",
            .noHistoryFound: "No Historical Jobs Found"
        ],
        .german: [
            .sectionOperations: "Operationen",
            .sectionDiagnostics: "Diagnose",
            .sectionAudit: "Prüfung",
            .tabSetup: "Einrichtung",
            .tabLiveTransfer: "Live-Transfer",
            .tabPerformance: "Leistung",
            .tabErrorCenter: "Fehlerzentrum",
            .tabHistory: "Verlauf & Berichte",
            .refreshDisksHelp: "Verbundene Laufwerke aktualisieren",
            .languageMenuTitle: "Sprache (18 Sprachen)",
            .setupTitle: "Transfer-Einrichtung",
            .setupSubtitle: "Quell- und Ziellaufwerk wählen, Richtlinie festlegen und Kapazität prüfen.",
            .sourceLocation: "Quellort",
            .destinationLocation: "Zielort",
            .advancedOptions: "Erweiterte Transferoptionen",
            .directIOToggle: "Direct I/O (OS-Cache über F_NOCACHE umgehen)",
            .readbackVerifyToggle: "Physische Rückleseprüfung (BLAKE3-Zielprüfung)",
            .memoryBufferBudget: "Speicherpuffer-Budget:",
            .startTransfer: "Transfer starten",
            .activeJob: "Aktiver Auftrag:",
            .noActiveTransfer: "Kein aktiver Transfer",
            .currentSpeed: "Aktuelle Geschwindigkeit",
            .estimatedTime: "Verbleibende Zeit (ETA)",
            .activeChunkEngine: "Aktive Chunk-Engine",
            .readWriteLatency: "Lese-/Schreiblatenz",
            .bufferUtilization: "Pufferauslastung",
            .quarantinedFiles: "Isolierte Dateien",
            .throughputLast60s: "Durchsatzaktivität (Letzte 60 Sekunden)",
            .pauseTransfer: "Pausieren",
            .resumeTransfer: "Fortsetzen",
            .cancelTransfer: "Abbrechen",
            .perfTitle: "Leistungs- & Engpass-Monitor",
            .perfSubtitle: "Echtzeit-Telemetrie des Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Quelllaufwerk (Lesen)",
            .destDeviceWrite: "Ziellaufwerk (Schreiben)",
            .throughputProfiles: "Durchsatzprofile (MB/s)",
            .adaptiveChunkLadder: "Adaptive Chunk-Stufenleiter",
            .errorCenterTitle: "Fehler-Quarantänezentrum",
            .errorCenterSubtitle: "Unlesbare Dateien werden isoliert, ohne den Transfer zu stoppen. Überspringen löscht niemals Daten.",
            .skipAllQuarantined: "Alle überspringen",
            .retryAll: "Alle wiederholen",
            .searchFilePath: "Dateipfad suchen...",
            .noQuarantinedFiles: "Keine isolierten Dateien",
            .healthyPipelineSubtitle: "Gesunde Pipeline: Alle Dateien erfolgreich verarbeitet.",
            .inspectAction: "Prüfen",
            .skipAction: "Überspringen",
            .retryAction: "Wiederholen",
            .historyTitle: "Auftragsverlauf & Prüfberichte",
            .historySubtitle: "Dauerhaftes Protokoll vergangener Transfers im SQLite-WAL-Journal.",
            .refreshButton: "Aktualisieren",
            .searchJobOrPath: "Auftrags-ID oder Pfad suchen...",
            .maskPersonalPaths: "Persönliche Pfade maskieren",
            .noHistoryFound: "Keine vergangenen Aufträge gefunden"
        ],
        .french: [
            .sectionOperations: "Opérations",
            .sectionDiagnostics: "Diagnostics",
            .sectionAudit: "Audit",
            .tabSetup: "Configuration",
            .tabLiveTransfer: "Transfert en direct",
            .tabPerformance: "Performance",
            .tabErrorCenter: "Centre d'erreurs",
            .tabHistory: "Historique & Rapports",
            .refreshDisksHelp: "Actualiser les disques connectés",
            .languageMenuTitle: "Langue (18 langues)",
            .setupTitle: "Configuration du transfert",
            .setupSubtitle: "Sélectionnez les volumes source et cible, choisissez la politique et vérifiez la capacité.",
            .sourceLocation: "Emplacement source",
            .destinationLocation: "Emplacement cible",
            .advancedOptions: "Options avancées de transfert",
            .directIOToggle: "E/S directes (contourner le cache OS via F_NOCACHE)",
            .readbackVerifyToggle: "Vérification physique par relecture (BLAKE3)",
            .memoryBufferBudget: "Budget mémoire tampon :",
            .startTransfer: "Démarrer le transfert",
            .activeJob: "Tâche active :",
            .noActiveTransfer: "Aucun transfert actif",
            .currentSpeed: "Vitesse actuelle",
            .estimatedTime: "Temps restant estimé (ETA)",
            .activeChunkEngine: "Moteur de blocs actif",
            .readWriteLatency: "Latence lecture / écriture",
            .bufferUtilization: "Utilisation du tampon",
            .quarantinedFiles: "Fichiers en quarantaine",
            .throughputLast60s: "Activité du débit (60 dernières secondes)",
            .pauseTransfer: "Mettre en pause",
            .resumeTransfer: "Reprendre",
            .cancelTransfer: "Annuler le transfert",
            .perfTitle: "Moniteur de performance et goulots",
            .perfSubtitle: "Télémétrie en temps réel Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Disque source (Lecture)",
            .destDeviceWrite: "Disque cible (Écriture)",
            .throughputProfiles: "Profils de débit (Mo/s)",
            .adaptiveChunkLadder: "Échelle adaptative des blocs",
            .errorCenterTitle: "Centre de quarantaine des erreurs",
            .errorCenterSubtitle: "Les fichiers illisibles sont isolés sans bloquer le transfert. Ignorer ne supprime aucune donnée.",
            .skipAllQuarantined: "Tout ignorer",
            .retryAll: "Tout réessayer",
            .searchFilePath: "Rechercher un chemin...",
            .noQuarantinedFiles: "Aucun fichier en quarantaine",
            .healthyPipelineSubtitle: "Pipeline sain : tous les fichiers ont été traités avec succès.",
            .inspectAction: "Inspecter",
            .skipAction: "Ignorer",
            .retryAction: "Réessayer",
            .historyTitle: "Historique & Rapports d'audit",
            .historySubtitle: "Archive durable des transferts stockée dans le journal SQLite WAL.",
            .refreshButton: "Actualiser",
            .searchJobOrPath: "Rechercher ID ou chemin...",
            .maskPersonalPaths: "Masquer les chemins personnels",
            .noHistoryFound: "Aucun historique trouvé"
        ],
        .spanish: [
            .sectionOperations: "Operaciones",
            .sectionDiagnostics: "Diagnóstico",
            .sectionAudit: "Auditoría",
            .tabSetup: "Configuración",
            .tabLiveTransfer: "Transferencia en vivo",
            .tabPerformance: "Rendimiento",
            .tabErrorCenter: "Centro de errores",
            .tabHistory: "Historial e informes",
            .refreshDisksHelp: "Actualizar discos conectados",
            .languageMenuTitle: "Idioma (18 idiomas)",
            .setupTitle: "Configuración de transferencia",
            .setupSubtitle: "Seleccione origen y destino, elija la política de transferencia y verifique la capacidad.",
            .sourceLocation: "Ubicación de origen",
            .destinationLocation: "Ubicación de destino",
            .advancedOptions: "Opciones avanzadas de transferencia",
            .directIOToggle: "E/S directa (omitir caché del SO mediante F_NOCACHE)",
            .readbackVerifyToggle: "Verificación física de lectura (BLAKE3)",
            .memoryBufferBudget: "Presupuesto de búfer de memoria:",
            .startTransfer: "Iniciar transferencia",
            .activeJob: "Tarea activa:",
            .noActiveTransfer: "Sin transferencia activa",
            .currentSpeed: "Velocidad actual",
            .estimatedTime: "Tiempo estimado (ETA)",
            .activeChunkEngine: "Motor de bloques activo",
            .readWriteLatency: "Latencia de lectura / escritura",
            .bufferUtilization: "Uso del búfer",
            .quarantinedFiles: "Archivos en cuarentena",
            .throughputLast60s: "Actividad de rendimiento (últimos 60 s)",
            .pauseTransfer: "Pausar",
            .resumeTransfer: "Reanudar",
            .cancelTransfer: "Cancelar",
            .perfTitle: "Monitor de rendimiento y cuellos de botella",
            .perfSubtitle: "Telemetría en tiempo real de Adaptive Disk Profiler (ADP) e Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Dispositivo de origen (Lectura)",
            .destDeviceWrite: "Dispositivo de destino (Escritura)",
            .throughputProfiles: "Perfiles de velocidad (MB/s)",
            .adaptiveChunkLadder: "Escalera adaptativa de bloques",
            .errorCenterTitle: "Centro de cuarentena de errores",
            .errorCenterSubtitle: "Los archivos ilegibles se aíslan sin detener la copia. Omitir nunca borra datos.",
            .skipAllQuarantined: "Omitir todos",
            .retryAll: "Reintentar todos",
            .searchFilePath: "Buscar ruta de archivo...",
            .noQuarantinedFiles: "Sin archivos en cuarentena",
            .healthyPipelineSubtitle: "Flujo saludable: todos los archivos se procesaron correctamente.",
            .inspectAction: "Inspeccionar",
            .skipAction: "Omitir",
            .retryAction: "Reintentar",
            .historyTitle: "Historial e informes de auditoría",
            .historySubtitle: "Registro duradero en el diario SQLite WAL.",
            .refreshButton: "Actualizar",
            .searchJobOrPath: "Buscar ID o ruta...",
            .maskPersonalPaths: "Ocultar rutas personales",
            .noHistoryFound: "No se encontraron tareas anteriores"
        ],
        .italian: [
            .sectionOperations: "Operazioni",
            .sectionDiagnostics: "Diagnostica",
            .sectionAudit: "Audit",
            .tabSetup: "Configurazione",
            .tabLiveTransfer: "Trasferimento dal vivo",
            .tabPerformance: "Prestazioni",
            .tabErrorCenter: "Centro errori",
            .tabHistory: "Cronologia e report",
            .refreshDisksHelp: "Aggiorna dischi connessi",
            .languageMenuTitle: "Lingua (18 lingue)",
            .setupTitle: "Configurazione trasferimento",
            .setupSubtitle: "Seleziona i volumi di origine e destinazione, scegli la politica e verifica la capacità.",
            .sourceLocation: "Posizione origine",
            .destinationLocation: "Posizione destinazione",
            .advancedOptions: "Opzioni avanzate di trasferimento",
            .directIOToggle: "I/O diretto (ignora cache OS tramite F_NOCACHE)",
            .readbackVerifyToggle: "Verifica fisica di rilettura (BLAKE3)",
            .memoryBufferBudget: "Budget buffer di memoria:",
            .startTransfer: "Avvia trasferimento",
            .activeJob: "Lavoro attivo:",
            .noActiveTransfer: "Nessun trasferimento attivo",
            .currentSpeed: "Velocità attuale",
            .estimatedTime: "Tempo stimato (ETA)",
            .activeChunkEngine: "Motore chunk attivo",
            .readWriteLatency: "Latenza lettura / scrittura",
            .bufferUtilization: "Utilizzo buffer",
            .quarantinedFiles: "File in quarantena",
            .throughputLast60s: "Attività throughput (ultimi 60 secondi)",
            .pauseTransfer: "Metti in pausa",
            .resumeTransfer: "Riprendi",
            .cancelTransfer: "Annulla",
            .perfTitle: "Monitor prestazioni e colli di bottiglia",
            .perfSubtitle: "Telemetria in tempo reale Adaptive Disk Profiler (ADP) e Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Dispositivo origine (Lettura)",
            .destDeviceWrite: "Dispositivo destinazione (Scrittura)",
            .throughputProfiles: "Profili di velocità (MB/s)",
            .adaptiveChunkLadder: "Scala adattiva dei chunk",
            .errorCenterTitle: "Centro quarantena errori",
            .errorCenterSubtitle: "I file illeggibili vengono isolati senza bloccare la copia. Saltare non elimina mai i dati.",
            .skipAllQuarantined: "Salta tutti",
            .retryAll: "Riprova tutti",
            .searchFilePath: "Cerca percorso file...",
            .noQuarantinedFiles: "Nessun file in quarantena",
            .healthyPipelineSubtitle: "Pipeline integra: tutti i file sono stati elaborati.",
            .inspectAction: "Ispeziona",
            .skipAction: "Salta",
            .retryAction: "Riprova",
            .historyTitle: "Cronologia lavori e report",
            .historySubtitle: "Archivio persistente salvato nel registro SQLite WAL.",
            .refreshButton: "Aggiorna",
            .searchJobOrPath: "Cerca ID o percorso...",
            .maskPersonalPaths: "Maschera percorsi personali",
            .noHistoryFound: "Nessuna cronologia trovata"
        ],
        .portuguese: [
            .sectionOperations: "Operações",
            .sectionDiagnostics: "Diagnóstico",
            .sectionAudit: "Auditoria",
            .tabSetup: "Configuração",
            .tabLiveTransfer: "Transferência ao Vivo",
            .tabPerformance: "Desempenho",
            .tabErrorCenter: "Central de Erros",
            .tabHistory: "Histórico e Relatórios",
            .refreshDisksHelp: "Atualizar discos conectados",
            .languageMenuTitle: "Idioma (18 idiomas)",
            .setupTitle: "Configuração da Transferência",
            .setupSubtitle: "Selecione os volumes de origem e destino, escolha a política e verifique a capacidade.",
            .sourceLocation: "Local de Origem",
            .destinationLocation: "Local de Destino",
            .advancedOptions: "Opções Avançadas de Transferência",
            .directIOToggle: "E/S Direta (ignorar cache do SO via F_NOCACHE)",
            .readbackVerifyToggle: "Verificação Física de Releitura (BLAKE3)",
            .memoryBufferBudget: "Orçamento de Buffer de Memória:",
            .startTransfer: "Iniciar Transferência",
            .activeJob: "Tarefa Ativa:",
            .noActiveTransfer: "Nenhuma Transferência Ativa",
            .currentSpeed: "Velocidade Atual",
            .estimatedTime: "Tempo Estimado (ETA)",
            .activeChunkEngine: "Motor de Blocos Ativo",
            .readWriteLatency: "Latência de Leitura / Gravação",
            .bufferUtilization: "Uso do Buffer",
            .quarantinedFiles: "Arquivos em Quarentena",
            .throughputLast60s: "Atividade de Taxa (Últimos 60 Segundos)",
            .pauseTransfer: "Pausar",
            .resumeTransfer: "Retomar",
            .cancelTransfer: "Cancelar",
            .perfTitle: "Monitor de Desempenho e Gargalos",
            .perfSubtitle: "Telemetria em tempo real do Adaptive Disk Profiler (ADP) e Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Dispositivo de Origem (Leitura)",
            .destDeviceWrite: "Dispositivo de Destino (Gravação)",
            .throughputProfiles: "Perfis de Taxa (MB/s)",
            .adaptiveChunkLadder: "Escala Adaptativa de Blocos",
            .errorCenterTitle: "Central de Quarentena de Erros",
            .errorCenterSubtitle: "Arquivos ilegíveis são isolados sem travar a transferência. Pular nunca apaga dados.",
            .skipAllQuarantined: "Pular Todos",
            .retryAll: "Tentar Todos Novamente",
            .searchFilePath: "Pesquisar caminho...",
            .noQuarantinedFiles: "Nenhum Arquivo em Quarentena",
            .healthyPipelineSubtitle: "Fluxo saudável: todos os arquivos processados com sucesso.",
            .inspectAction: "Inspecionar",
            .skipAction: "Pular",
            .retryAction: "Tentar Novamente",
            .historyTitle: "Histórico e Relatórios de Auditoria",
            .historySubtitle: "Registro durável de transferências no banco SQLite WAL.",
            .refreshButton: "Atualizar",
            .searchJobOrPath: "Pesquisar ID ou caminho...",
            .maskPersonalPaths: "Mascarar Caminhos Pessoais",
            .noHistoryFound: "Nenhum Histórico Encontrado"
        ],
        .russian: [
            .sectionOperations: "Операции",
            .sectionDiagnostics: "Диагностика",
            .sectionAudit: "Аудит",
            .tabSetup: "Настройка",
            .tabLiveTransfer: "Активная передача",
            .tabPerformance: "Производительность",
            .tabErrorCenter: "Центр ошибок",
            .tabHistory: "История и отчёты",
            .refreshDisksHelp: "Обновить подключённые диски",
            .languageMenuTitle: "Язык (18 языков)",
            .setupTitle: "Настройка копирования",
            .setupSubtitle: "Выберите источник и назначение, укажите политику и проверьте ёмкость.",
            .sourceLocation: "Исходный диск / папка",
            .destinationLocation: "Целевой диск / папка",
            .advancedOptions: "Дополнительные параметры",
            .directIOToggle: "Прямой ввод-вывод (обход кэша ОС через F_NOCACHE)",
            .readbackVerifyToggle: "Физическая проверка чтения (BLAKE3)",
            .memoryBufferBudget: "Лимит буфера памяти:",
            .startTransfer: "Начать передачу",
            .activeJob: "Активная задача:",
            .noActiveTransfer: "Нет активных задач",
            .currentSpeed: "Текущая скорость",
            .estimatedTime: "Оставшееся время (ETA)",
            .activeChunkEngine: "Размер блока (Chunk)",
            .readWriteLatency: "Задержка чтения / записи",
            .bufferUtilization: "Загрузка буфера",
            .quarantinedFiles: "Файлы в карантине",
            .throughputLast60s: "График скорости (последние 60 сек)",
            .pauseTransfer: "Пауза",
            .resumeTransfer: "Продолжить",
            .cancelTransfer: "Отменить",
            .perfTitle: "Мониторинг производительности и узких мест",
            .perfSubtitle: "Телеметрия Adaptive Disk Profiler (ADP) и Intelligent Transfer Optimizer (ITO) в реальном времени.",
            .sourceDeviceRead: "Источник (Чтение)",
            .destDeviceWrite: "Назначение (Запись)",
            .throughputProfiles: "Профили пропускной способности (МБ/с)",
            .adaptiveChunkLadder: "Адаптивная шкала блоков",
            .errorCenterTitle: "Центр карантина ошибок",
            .errorCenterSubtitle: "Нечитаемые файлы изолируются без остановки копирования. Пропуск не удаляет данные.",
            .skipAllQuarantined: "Пропустить все",
            .retryAll: "Повторить все",
            .searchFilePath: "Поиск по пути файла...",
            .noQuarantinedFiles: "Нет файлов в карантине",
            .healthyPipelineSubtitle: "Все файлы успешно обработаны.",
            .inspectAction: "Детали",
            .skipAction: "Пропустить",
            .retryAction: "Повторить",
            .historyTitle: "История задач и отчёты",
            .historySubtitle: "Надёжный журнал передач в базе SQLite WAL.",
            .refreshButton: "Обновить",
            .searchJobOrPath: "Поиск по ID или пути...",
            .maskPersonalPaths: "Скрывать личные пути",
            .noHistoryFound: "История задач пуста"
        ],
        .japanese: [
            .sectionOperations: "操作",
            .sectionDiagnostics: "診断・分析",
            .sectionAudit: "監査ログ",
            .tabSetup: "転送設定",
            .tabLiveTransfer: "ライブ転送",
            .tabPerformance: "パフォーマンス",
            .tabErrorCenter: "エラーセンター",
            .tabHistory: "履歴とレポート",
            .refreshDisksHelp: "接続ディスクを更新",
            .languageMenuTitle: "言語 (18言語)",
            .setupTitle: "転送セットアップ",
            .setupSubtitle: "転送元と転送先を選択し、転送ポリシーと空き容量を確認します。",
            .sourceLocation: "転送元ロケーション",
            .destinationLocation: "転送先ロケーション",
            .advancedOptions: "詳細転送オプション",
            .directIOToggle: "ダイレクトI/O (F_NOCACHEによりOSキャッシュを回避)",
            .readbackVerifyToggle: "物理リードバック検証 (BLAKE3ハッシュ照合)",
            .memoryBufferBudget: "メモリバッファ上限:",
            .startTransfer: "転送を開始",
            .activeJob: "実行中ジョブ:",
            .noActiveTransfer: "実行中の転送なし",
            .currentSpeed: "現在速度",
            .estimatedTime: "残り時間 (ETA)",
            .activeChunkEngine: "チャンクエンジン",
            .readWriteLatency: "読込 / 書込レイテンシ",
            .bufferUtilization: "バッファ使用率",
            .quarantinedFiles: "隔離ファイル数",
            .throughputLast60s: "スループット推移 (直近60秒)",
            .pauseTransfer: "一時停止",
            .resumeTransfer: "再開",
            .cancelTransfer: "キャンセル",
            .perfTitle: "パフォーマンス＆ボトルネック監視",
            .perfSubtitle: "Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO) リアルタイム計測。",
            .sourceDeviceRead: "転送元ディスク (読込)",
            .destDeviceWrite: "転送先ディスク (書込)",
            .throughputProfiles: "スループットプロファイル (MB/s)",
            .adaptiveChunkLadder: "アダプティブ・チャンクサイズ階段",
            .errorCenterTitle: "エラー隔離センター",
            .errorCenterSubtitle: "読み取り不能なファイルは転送を止めずに隔離されます。スキップしてもデータは削除されません。",
            .skipAllQuarantined: "すべてスキップ",
            .retryAll: "すべて再試行",
            .searchFilePath: "ファイルパスを検索...",
            .noQuarantinedFiles: "隔離されたファイルはありません",
            .healthyPipelineSubtitle: "すべてのファイルが正常に処理されました。",
            .inspectAction: "詳細",
            .skipAction: "スキップ",
            .retryAction: "再試行",
            .historyTitle: "ジョブ履歴と監査レポート",
            .historySubtitle: "SQLite WALジャーナルに保存された転送履歴。",
            .refreshButton: "更新",
            .searchJobOrPath: "ジョブIDまたはパスを検索...",
            .maskPersonalPaths: "個人パスをマスクする",
            .noHistoryFound: "履歴データがありません"
        ],
        .korean: [
            .sectionOperations: "작업",
            .sectionDiagnostics: "진단 및 분석",
            .sectionAudit: "감사 기록",
            .tabSetup: "전송 설정",
            .tabLiveTransfer: "실시간 전송",
            .tabPerformance: "성능 모니터",
            .tabErrorCenter: "오류 센터",
            .tabHistory: "기록 및 보고서",
            .refreshDisksHelp: "연결된 디스크 새로고침",
            .languageMenuTitle: "언어 (18개 국어)",
            .setupTitle: "전송 설정",
            .setupSubtitle: "원본 및 대상 볼륨을 선택하고 전송 정책과 용량을 확인하세요.",
            .sourceLocation: "원본 위치",
            .destinationLocation: "대상 위치",
            .advancedOptions: "고급 전송 옵션",
            .directIOToggle: "직접 I/O (F_NOCACHE로 OS 캐시 우회)",
            .readbackVerifyToggle: "물리적 재읽기 무결성 검증 (BLAKE3)",
            .memoryBufferBudget: "메모리 버퍼 한도:",
            .startTransfer: "전송 시작",
            .activeJob: "활성 작업:",
            .noActiveTransfer: "진행 중인 전송 없음",
            .currentSpeed: "현재 속도",
            .estimatedTime: "예상 남은 시간 (ETA)",
            .activeChunkEngine: "활성 청크 엔진",
            .readWriteLatency: "읽기 / 쓰기 지연 시간",
            .bufferUtilization: "버퍼 사용률",
            .quarantinedFiles: "격리된 파일",
            .throughputLast60s: "처리량 추이 (최근 60초)",
            .pauseTransfer: "일시 정지",
            .resumeTransfer: "재개",
            .cancelTransfer: "전송 취소",
            .perfTitle: "성능 및 병목 현상 모니터",
            .perfSubtitle: "Adaptive Disk Profiler (ADP) 및 Intelligent Transfer Optimizer (ITO) 실시간 텔레메트리.",
            .sourceDeviceRead: "원본 장치 (읽기)",
            .destDeviceWrite: "대상 장치 (쓰기)",
            .throughputProfiles: "처리량 프로필 (MB/s)",
            .adaptiveChunkLadder: "적응형 청크 크기 단계",
            .errorCenterTitle: "오류 격리 센터",
            .errorCenterSubtitle: "읽을 수 없는 파일은 전송을 중단하지 않고 격리됩니다. 건너뛰기는 데이터를 삭제하지 않습니다.",
            .skipAllQuarantined: "모두 건너뛰기",
            .retryAll: "모두 재시도",
            .searchFilePath: "파일 경로 검색...",
            .noQuarantinedFiles: "격리된 파일 없음",
            .healthyPipelineSubtitle: "모든 파일이 정상적으로 처리되었습니다.",
            .inspectAction: "검사",
            .skipAction: "건너뛰기",
            .retryAction: "재시도",
            .historyTitle: "작업 기록 및 감사 보고서",
            .historySubtitle: "SQLite WAL 저널에 저장된 전송 기록.",
            .refreshButton: "새로고침",
            .searchJobOrPath: "작업 ID 또는 경로 검색...",
            .maskPersonalPaths: "개인 경로 마스킹",
            .noHistoryFound: "작업 기록 없음"
        ],
        .chineseSimplified: [
            .sectionOperations: "操作",
            .sectionDiagnostics: "诊断与分析",
            .sectionAudit: "审计与报告",
            .tabSetup: "传输设置",
            .tabLiveTransfer: "实时传输",
            .tabPerformance: "性能监控",
            .tabErrorCenter: "错误中心",
            .tabHistory: "历史与报告",
            .refreshDisksHelp: "刷新已连接的磁盘设备",
            .languageMenuTitle: "语言 (18种语言)",
            .setupTitle: "传输配置",
            .setupSubtitle: "选择源卷与目标卷，配置传输策略并验证磁盘容量。",
            .sourceLocation: "源位置",
            .destinationLocation: "目标位置",
            .advancedOptions: "高级传输选项",
            .directIOToggle: "直接 I/O (通过 F_NOCACHE 绕过系统缓存)",
            .readbackVerifyToggle: "物理回读校验 (BLAKE3 校验目标字节)",
            .memoryBufferBudget: "内存缓冲区预算:",
            .startTransfer: "开始传输",
            .activeJob: "当前任务:",
            .noActiveTransfer: "无活动传输",
            .currentSpeed: "当前速度",
            .estimatedTime: "预计剩余时间 (ETA)",
            .activeChunkEngine: "活动分块引擎",
            .readWriteLatency: "读 / 写延迟",
            .bufferUtilization: "缓冲区使用率",
            .quarantinedFiles: "隔离文件",
            .throughputLast60s: "吞吐量活动 (最近 60 秒)",
            .pauseTransfer: "暂停传输",
            .resumeTransfer: "继续传输",
            .cancelTransfer: "取消传输",
            .perfTitle: "性能与瓶颈监控",
            .perfSubtitle: "自适应磁盘分析器 (ADP) 与智能传输优化器 (ITO) 实时遥测。",
            .sourceDeviceRead: "源设备 (读取)",
            .destDeviceWrite: "目标设备 (写入)",
            .throughputProfiles: "吞吐量曲线 (MB/s)",
            .adaptiveChunkLadder: "自适应分块大小阶梯",
            .errorCenterTitle: "错误隔离中心",
            .errorCenterSubtitle: "无法读取的文件将被隔离而不会中断传输。跳过文件绝不会删除任何数据。",
            .skipAllQuarantined: "全部跳过",
            .retryAll: "全部重试",
            .searchFilePath: "搜索文件路径...",
            .noQuarantinedFiles: "无隔离文件",
            .healthyPipelineSubtitle: "传输管线健康：所有文件均已成功处理。",
            .inspectAction: "详情",
            .skipAction: "跳过",
            .retryAction: "重试",
            .historyTitle: "任务历史与审计报告",
            .historySubtitle: "保存在 SQLite WAL 日志中的持久化传输记录。",
            .refreshButton: "刷新",
            .searchJobOrPath: "搜索任务 ID 或路径...",
            .maskPersonalPaths: "隐藏个人路径",
            .noHistoryFound: "未找到历史任务"
        ],
        .chineseTraditional: [
            .sectionOperations: "操作",
            .sectionDiagnostics: "診斷與分析",
            .sectionAudit: "稽核與報告",
            .tabSetup: "傳輸設定",
            .tabLiveTransfer: "即時傳輸",
            .tabPerformance: "效能監控",
            .tabErrorCenter: "錯誤中心",
            .tabHistory: "歷史與報告",
            .refreshDisksHelp: "重新整理已連接的磁碟",
            .languageMenuTitle: "語言 (18種語言)",
            .setupTitle: "傳輸設定",
            .setupSubtitle: "選擇來源與目標磁碟區，選擇傳輸原則並驗證容量。",
            .sourceLocation: "來源位置",
            .destinationLocation: "目標位置",
            .advancedOptions: "進階傳輸選項",
            .directIOToggle: "直接 I/O (透過 F_NOCACHE 繞過系統快取)",
            .readbackVerifyToggle: "實體回讀驗證 (BLAKE3 驗證目標位元組)",
            .memoryBufferBudget: "記憶體緩衝區預算:",
            .startTransfer: "開始傳輸",
            .activeJob: "執行中工作:",
            .noActiveTransfer: "無執行中傳輸",
            .currentSpeed: "目前速度",
            .estimatedTime: "預估剩餘時間 (ETA)",
            .activeChunkEngine: "活動區塊引擎",
            .readWriteLatency: "讀取 / 寫入延遲",
            .bufferUtilization: "緩衝區使用率",
            .quarantinedFiles: "隔離檔案",
            .throughputLast60s: "吞吐量活動 (最近 60 秒)",
            .pauseTransfer: "暫停傳輸",
            .resumeTransfer: "繼續傳輸",
            .cancelTransfer: "取消傳輸",
            .perfTitle: "效能與瓶頸監控",
            .perfSubtitle: "自適應磁碟分析器 (ADP) 與智慧傳輸最佳化工具 (ITO) 即時遙測。",
            .sourceDeviceRead: "來源裝置 (讀取)",
            .destDeviceWrite: "目標裝置 (寫入)",
            .throughputProfiles: "吞吐量剖析 (MB/s)",
            .adaptiveChunkLadder: "自適應區塊大小階梯",
            .errorCenterTitle: "錯誤隔離中心",
            .errorCenterSubtitle: "無法讀取的檔案會被隔離而不中斷傳輸。略過檔案絕不會刪除任何資料。",
            .skipAllQuarantined: "全部略過",
            .retryAll: "全部重試",
            .searchFilePath: "搜尋檔案路徑...",
            .noQuarantinedFiles: "無隔離檔案",
            .healthyPipelineSubtitle: "傳輸管線健康：所有檔案均已成功處理。",
            .inspectAction: "檢查",
            .skipAction: "略過",
            .retryAction: "重試",
            .historyTitle: "工作歷史與稽核報告",
            .historySubtitle: "儲存在 SQLite WAL 日誌中的持久傳輸記錄。",
            .refreshButton: "重新整理",
            .searchJobOrPath: "搜尋工作 ID 或路徑...",
            .maskPersonalPaths: "遮蔽個人路徑",
            .noHistoryFound: "找不到歷史工作"
        ],
        .arabic: [
            .sectionOperations: "العمليات",
            .sectionDiagnostics: "التشخيص",
            .sectionAudit: "التدقيق",
            .tabSetup: "الإعداد",
            .tabLiveTransfer: "النقل المباشر",
            .tabPerformance: "الأداء",
            .tabErrorCenter: "مركز الأخطاء",
            .tabHistory: "السجل والتقارير",
            .refreshDisksHelp: "تحديث الأقراص المتصلة",
            .languageMenuTitle: "اللغة (18 لغة)",
            .setupTitle: "إعداد النقل",
            .setupSubtitle: "حدد وحدات التخزين المصدر والوجهة، واختر سياسة النقل وتحقق من السعة.",
            .sourceLocation: "موقع المصدر",
            .destinationLocation: "موقع الوجهة",
            .advancedOptions: "خيارات النقل المتقدمة",
            .directIOToggle: "إدخال/إخراج مباشر (تجاوز ذاكرة النظام عبر F_NOCACHE)",
            .readbackVerifyToggle: "التحقق الفيزيائي من القراءة (BLAKE3)",
            .memoryBufferBudget: "ميزانية الذاكرة المؤقتة:",
            .startTransfer: "بدء النقل",
            .activeJob: "المهمة النشطة:",
            .noActiveTransfer: "لا يوجد نقل نشط",
            .currentSpeed: "السرعة الحالية",
            .estimatedTime: "الوقت المتبقي (ETA)",
            .activeChunkEngine: "محرك القطع النشط",
            .readWriteLatency: "زمن استجابة القراءة / الكتابة",
            .bufferUtilization: "استخدام الذاكرة المؤقتة",
            .quarantinedFiles: "الملفات المعزولة",
            .throughputLast60s: "نشاط السرعة (آخر 60 ثانية)",
            .pauseTransfer: "إيقاف مؤقت",
            .resumeTransfer: "استئناف",
            .cancelTransfer: "إلغاء النقل",
            .perfTitle: "مراقب الأداء والاختناقات",
            .perfSubtitle: "القياس عن بُعد في الوقت الفعلي لـ ADP و ITO.",
            .sourceDeviceRead: "جهاز المصدر (قراءة)",
            .destDeviceWrite: "جهاز الوجهة (كتابة)",
            .throughputProfiles: "ملفات تعريف السرعة (ميجابايت/ث)",
            .adaptiveChunkLadder: "سلم حجم القطع التكيفي",
            .errorCenterTitle: "مركز عزل الأخطاء",
            .errorCenterSubtitle: "يتم عزل الملفات غير القابلة للقراءة دون إيقاف النقل. التخطي لا يحذف أي بيانات أبداً.",
            .skipAllQuarantined: "تخطي الكل",
            .retryAll: "إعادة محاولة الكل",
            .searchFilePath: "البحث في مسار الملف...",
            .noQuarantinedFiles: "لا توجد ملفات معزولة",
            .healthyPipelineSubtitle: "مسار نقل سليم: تمت معالجة جميع الملفات بنجاح.",
            .inspectAction: "فحص",
            .skipAction: "تخطي",
            .retryAction: "إعادة المحاولة",
            .historyTitle: "سجل المهام وتقارير التدقيق",
            .historySubtitle: "سجل دائم لمهام النقل السابقة في قاعدة SQLite WAL.",
            .refreshButton: "تحديث",
            .searchJobOrPath: "البحث عن معرف المهمة أو المسار...",
            .maskPersonalPaths: "إخفاء المسارات الشخصية",
            .noHistoryFound: "لم يتم العثور على مهام سابقة"
        ],
        .hindi: [
            .sectionOperations: "संचालन",
            .sectionDiagnostics: "निदान",
            .sectionAudit: "ऑडिट",
            .tabSetup: "सेटअप",
            .tabLiveTransfer: "लाइव ट्रांसफ़र",
            .tabPerformance: "प्रदर्शन",
            .tabErrorCenter: "त्रुटि केंद्र",
            .tabHistory: "इतिहास और रिपोर्ट",
            .refreshDisksHelp: "कनेक्टेड डिस्क रीफ़्रेश करें",
            .languageMenuTitle: "भाषा (18 भाषाएँ)",
            .setupTitle: "ट्रांसफ़र सेटअप",
            .setupSubtitle: "स्रोत और गंतव्य डिस्क चुनें, नीति निर्धारित करें और क्षमता की जाँच करें।",
            .sourceLocation: "स्रोत स्थान",
            .destinationLocation: "गंतव्य स्थान",
            .advancedOptions: "उन्नत ट्रांसफ़र विकल्प",
            .directIOToggle: "डायरेक्ट I/O (F_NOCACHE द्वारा OS कैश बायपास)",
            .readbackVerifyToggle: "भौतिक रीडबैक सत्यापन (BLAKE3)",
            .memoryBufferBudget: "मेमोरी बफ़र बजट:",
            .startTransfer: "ट्रांसफ़र शुरू करें",
            .activeJob: "सक्रिय कार्य:",
            .noActiveTransfer: "कोई सक्रिय ट्रांसफ़र नहीं",
            .currentSpeed: "वर्तमान गति",
            .estimatedTime: "अनुमानित समय (ETA)",
            .activeChunkEngine: "सक्रिय चंक इंजन",
            .readWriteLatency: "रीड / राइट विलंबता",
            .bufferUtilization: "बफ़र उपयोग",
            .quarantinedFiles: "अलग की गई फ़ाइलें",
            .throughputLast60s: "थ्रूपुट गतिविधि (पिछले 60 सेकंड)",
            .pauseTransfer: "रोकें",
            .resumeTransfer: "फिर से शुरू करें",
            .cancelTransfer: "रद्द करें",
            .perfTitle: "प्रदर्शन और बॉटलनेक मॉनिटर",
            .perfSubtitle: "Adaptive Disk Profiler (ADP) और Intelligent Transfer Optimizer (ITO) लाइव टेलीमेट्री।",
            .sourceDeviceRead: "स्रोत डिवाइस (रीड)",
            .destDeviceWrite: "गंतव्य डिवाइस (राइट)",
            .throughputProfiles: "थ्रूपुट प्रोफ़ाइल (MB/s)",
            .adaptiveChunkLadder: "अनुकूली चंक आकार सीढ़ी",
            .errorCenterTitle: "त्रुटि संगरोध केंद्र",
            .errorCenterSubtitle: "अपठनीय फ़ाइलों को ट्रांसफ़र रोके बिना अलग किया जाता है। छोड़ने से कोई डेटा नष्ट नहीं होता।",
            .skipAllQuarantined: "सभी छोड़ें",
            .retryAll: "सभी पुनः प्रयास करें",
            .searchFilePath: "फ़ाइल पथ खोजें...",
            .noQuarantinedFiles: "कोई अलग की गई फ़ाइल नहीं",
            .healthyPipelineSubtitle: "सभी फ़ाइलें सफलतापूर्वक संसाधित की गईं।",
            .inspectAction: "निरीक्षण",
            .skipAction: "छोड़ें",
            .retryAction: "पुनः प्रयास",
            .historyTitle: "कार्य इतिहास और ऑडिट रिपोर्ट",
            .historySubtitle: "SQLite WAL जर्नल में संग्रहीत स्थायी रिकॉर्ड।",
            .refreshButton: "रीफ़्रेश",
            .searchJobOrPath: "कार्य ID या पथ खोजें...",
            .maskPersonalPaths: "व्यक्तिगत पथ छुपाएँ",
            .noHistoryFound: "कोई पिछला कार्य नहीं मिला"
        ],
        .dutch: [
            .sectionOperations: "Bewerkingen",
            .sectionDiagnostics: "Diagnostiek",
            .sectionAudit: "Audit",
            .tabSetup: "Instellingen",
            .tabLiveTransfer: "Live Overdracht",
            .tabPerformance: "Prestaties",
            .tabErrorCenter: "Foutencentrum",
            .tabHistory: "Geschiedenis & Rapporten",
            .refreshDisksHelp: "Verbonden schijven vernieuwen",
            .languageMenuTitle: "Taal (18 talen)",
            .setupTitle: "Overdracht Instellen",
            .setupSubtitle: "Selecteer bron- en doelvolumes, kies het beleid en controleer de capaciteit.",
            .sourceLocation: "Bronlocatie",
            .destinationLocation: "Doellocatie",
            .advancedOptions: "Geavanceerde Opties",
            .directIOToggle: "Directe I/O (OS-cache omzeilen via F_NOCACHE)",
            .readbackVerifyToggle: "Fysieke Terugleesverificatie (BLAKE3)",
            .memoryBufferBudget: "Geheugenbufferbudget:",
            .startTransfer: "Overdracht Starten",
            .activeJob: "Actieve Taak:",
            .noActiveTransfer: "Geen Actieve Overdracht",
            .currentSpeed: "Huidige Snelheid",
            .estimatedTime: "Resterende Tijd (ETA)",
            .activeChunkEngine: "Actieve Chunk-Engine",
            .readWriteLatency: "Lees- / Schrijflatentie",
            .bufferUtilization: "Buffergebruik",
            .quarantinedFiles: "Bestanden in Quarantaine",
            .throughputLast60s: "Doorvoeractiviteit (Laatste 60 sec)",
            .pauseTransfer: "Pauzeren",
            .resumeTransfer: "Hervatten",
            .cancelTransfer: "Annuleren",
            .perfTitle: "Prestatie- & Knelpuntmonitor",
            .perfSubtitle: "Realtime telemetrie van Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Bronapparaat (Lezen)",
            .destDeviceWrite: "Doelapparaat (Schrijven)",
            .throughputProfiles: "Doorvoerprofielen (MB/s)",
            .adaptiveChunkLadder: "Adaptieve Chunk-ladder",
            .errorCenterTitle: "Quarantainecentrum voor Fouten",
            .errorCenterSubtitle: "Onleesbare bestanden worden geïsoleerd zonder de overdracht te stoppen. Overslaan verwijdert nooit gegevens.",
            .skipAllQuarantined: "Alles Overslaan",
            .retryAll: "Alles Opnieuw Proberen",
            .searchFilePath: "Bestandspad zoeken...",
            .noQuarantinedFiles: "Geen Bestanden in Quarantaine",
            .healthyPipelineSubtitle: "Alle bestanden zijn succesvol verwerkt.",
            .inspectAction: "Inspecteren",
            .skipAction: "Overslaan",
            .retryAction: "Opnieuw",
            .historyTitle: "Taakgeschiedenis & Auditrapporten",
            .historySubtitle: "Duurzaam archief opgeslagen in het SQLite WAL-journaal.",
            .refreshButton: "Vernieuwen",
            .searchJobOrPath: "Zoek taak-ID of pad...",
            .maskPersonalPaths: "Persoonlijke Paden Maskeren",
            .noHistoryFound: "Geen Geschiedenis Gevonden"
        ],
        .polish: [
            .sectionOperations: "Operacje",
            .sectionDiagnostics: "Diagnostyka",
            .sectionAudit: "Audyt",
            .tabSetup: "Konfiguracja",
            .tabLiveTransfer: "Transfer na żywo",
            .tabPerformance: "Wydajność",
            .tabErrorCenter: "Centrum błędów",
            .tabHistory: "Historia i raporty",
            .refreshDisksHelp: "Odśwież podłączone dyski",
            .languageMenuTitle: "Język (18 języków)",
            .setupTitle: "Konfiguracja transferu",
            .setupSubtitle: "Wybierz dysk źródłowy i docelowy, określ politykę oraz sprawdź pojemność.",
            .sourceLocation: "Lokalizacja źródłowa",
            .destinationLocation: "Lokalizacja docelowa",
            .advancedOptions: "Zaawansowane opcje transferu",
            .directIOToggle: "Bezpośrednie I/O (pomiń pamięć podręczną OS przez F_NOCACHE)",
            .readbackVerifyToggle: "Fizyczna weryfikacja odczytu (BLAKE3)",
            .memoryBufferBudget: "Budżet bufora pamięci:",
            .startTransfer: "Rozpocznij transfer",
            .activeJob: "Aktywne zadanie:",
            .noActiveTransfer: "Brak aktywnego transferu",
            .currentSpeed: "Aktualna prędkość",
            .estimatedTime: "Szacowany czas (ETA)",
            .activeChunkEngine: "Aktywny silnik bloków",
            .readWriteLatency: "Opóźnienie odczytu / zapisu",
            .bufferUtilization: "Użycie bufora",
            .quarantinedFiles: "Pliki w kwarantannie",
            .throughputLast60s: "Aktywność przepustowości (ostatnie 60 s)",
            .pauseTransfer: "Wstrzymaj",
            .resumeTransfer: "Wznów",
            .cancelTransfer: "Anuluj",
            .perfTitle: "Monitor wydajności i wąskich gardeł",
            .perfSubtitle: "Telemetria w czasie rzeczywistym Adaptive Disk Profiler (ADP) i Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Urządzenie źródłowe (Odczyt)",
            .destDeviceWrite: "Urządzenie docelowe (Zapis)",
            .throughputProfiles: "Profile przepustowości (MB/s)",
            .adaptiveChunkLadder: "Adaptacyjna drabina bloków",
            .errorCenterTitle: "Centrum kwarantanny błędów",
            .errorCenterSubtitle: "Nieczytelne pliki są izolowane bez zatrzymywania kopiowania. Pominięcie nigdy nie usuwa danych.",
            .skipAllQuarantined: "Pomiń wszystkie",
            .retryAll: "Ponów wszystkie",
            .searchFilePath: "Szukaj ścieżki pliku...",
            .noQuarantinedFiles: "Brak plików w kwarantannie",
            .healthyPipelineSubtitle: "Wszystkie pliki zostały pomyślnie przetworzone.",
            .inspectAction: "Sprawdź",
            .skipAction: "Pomiń",
            .retryAction: "Ponów",
            .historyTitle: "Historia zadań i raporty audytu",
            .historySubtitle: "Trwały rejestr zapisany w dzienniku SQLite WAL.",
            .refreshButton: "Odśwież",
            .searchJobOrPath: "Szukaj ID lub ścieżki...",
            .maskPersonalPaths: "Maskuj ścieżki prywatne",
            .noHistoryFound: "Brak historii zadań"
        ],
        .swedish: [
            .sectionOperations: "Operationer",
            .sectionDiagnostics: "Diagnostik",
            .sectionAudit: "Granskning",
            .tabSetup: "Inställningar",
            .tabLiveTransfer: "Live-överföring",
            .tabPerformance: "Prestanda",
            .tabErrorCenter: "Felcenter",
            .tabHistory: "Historik & Rapporter",
            .refreshDisksHelp: "Uppdatera anslutna diskar",
            .languageMenuTitle: "Språk (18 språk)",
            .setupTitle: "Överföringsinställning",
            .setupSubtitle: "Välj käll- och målvolym, ange policy och verifiera kapacitet.",
            .sourceLocation: "Källplats",
            .destinationLocation: "Målplats",
            .advancedOptions: "Avancerade överföringsalternativ",
            .directIOToggle: "Direkt I/O (kringgå OS-cache via F_NOCACHE)",
            .readbackVerifyToggle: "Fysisk återläsningsverifiering (BLAKE3)",
            .memoryBufferBudget: "Minnesbuffertbudget:",
            .startTransfer: "Starta överföring",
            .activeJob: "Aktivt jobb:",
            .noActiveTransfer: "Ingen aktiv överföring",
            .currentSpeed: "Aktuell hastighet",
            .estimatedTime: "Beräknad tid kvar (ETA)",
            .activeChunkEngine: "Aktiv blockmotor",
            .readWriteLatency: "Läs- / skrivlatens",
            .bufferUtilization: "Buffertanvändning",
            .quarantinedFiles: "Filer i karantän",
            .throughputLast60s: "Genomströmning (senaste 60 s)",
            .pauseTransfer: "Pausa",
            .resumeTransfer: "Återuppta",
            .cancelTransfer: "Avbryt",
            .perfTitle: "Prestanda- & Flaskhalsmonitor",
            .perfSubtitle: "Realtidstelemetri från Adaptive Disk Profiler (ADP) & Intelligent Transfer Optimizer (ITO).",
            .sourceDeviceRead: "Källenhet (Läs)",
            .destDeviceWrite: "Målenhet (Skriv)",
            .throughputProfiles: "Hastighetsprofiler (MB/s)",
            .adaptiveChunkLadder: "Adaptiv blockstege",
            .errorCenterTitle: "Felkarantäncenter",
            .errorCenterSubtitle: "Oläsbara filer isoleras utan att stoppa överföringen. Att hoppa över raderar aldrig data.",
            .skipAllQuarantined: "Hoppa över alla",
            .retryAll: "Försök alla igen",
            .searchFilePath: "Sök filsökväg...",
            .noQuarantinedFiles: "Inga filer i karantän",
            .healthyPipelineSubtitle: "Alla filer har behandlats framgångsrikt.",
            .inspectAction: "Inspektera",
            .skipAction: "Hoppa över",
            .retryAction: "Försök igen",
            .historyTitle: "Jobbhistorik & Granskningsrapporter",
            .historySubtitle: "Beständig historik lagrad i SQLite WAL-journal.",
            .refreshButton: "Uppdatera",
            .searchJobOrPath: "Sök jobb-ID eller sökväg...",
            .maskPersonalPaths: "Maskera personliga sökvägar",
            .noHistoryFound: "Ingen historik hittades"
        ],
        .ukrainian: [
            .sectionOperations: "Операції",
            .sectionDiagnostics: "Діагностика",
            .sectionAudit: "Аудит",
            .tabSetup: "Налаштування",
            .tabLiveTransfer: "Активна передача",
            .tabPerformance: "Продуктивність",
            .tabErrorCenter: "Центр помилок",
            .tabHistory: "Історія та звіти",
            .refreshDisksHelp: "Оновити підключені диски",
            .languageMenuTitle: "Мова (18 мов)",
            .setupTitle: "Налаштування копіювання",
            .setupSubtitle: "Оберіть вихідний і цільовий томи, політику передачі та перевірте ємність.",
            .sourceLocation: "Вихідне розташування",
            .destinationLocation: "Цільове розташування",
            .advancedOptions: "Додаткові параметри передачі",
            .directIOToggle: "Прямий ввід-вивід (обхід кешу ОС через F_NOCACHE)",
            .readbackVerifyToggle: "Фізична перевірка читанням (BLAKE3)",
            .memoryBufferBudget: "Бюджет буфера пам'яті:",
            .startTransfer: "Почати передачу",
            .activeJob: "Активне завдання:",
            .noActiveTransfer: "Немає активної передачі",
            .currentSpeed: "Поточна швидкість",
            .estimatedTime: "Залишок часу (ETA)",
            .activeChunkEngine: "Активний рушій блоків",
            .readWriteLatency: "Затримка читання / запису",
            .bufferUtilization: "Використання буфера",
            .quarantinedFiles: "Файли в карантині",
            .throughputLast60s: "Активність швидкості (останні 60 с)",
            .pauseTransfer: "Пауза",
            .resumeTransfer: "Продовжити",
            .cancelTransfer: "Скасувати",
            .perfTitle: "Монітор продуктивності та вузьких місць",
            .perfSubtitle: "Телеметрія Adaptive Disk Profiler (ADP) та Intelligent Transfer Optimizer (ITO) у реальному часі.",
            .sourceDeviceRead: "Вихідний пристрій (Читання)",
            .destDeviceWrite: "Цільовий пристрій (Запис)",
            .throughputProfiles: "Профілі пропускної здатності (МБ/с)",
            .adaptiveChunkLadder: "Адаптивна шкала блоків",
            .errorCenterTitle: "Центр карантину помилок",
            .errorCenterSubtitle: "Нечитабельні файли ізолюються без зупинки копіювання. Пропуск ніколи не видаляє дані.",
            .skipAllQuarantined: "Пропустити всі",
            .retryAll: "Повторити всі",
            .searchFilePath: "Пошук шляху файлу...",
            .noQuarantinedFiles: "Немає файлів у карантині",
            .healthyPipelineSubtitle: "Усі файли успішно оброблено.",
            .inspectAction: "Огляд",
            .skipAction: "Пропустити",
            .retryAction: "Повторити",
            .historyTitle: "Історія завдань та звіти аудиту",
            .historySubtitle: "Надійний запис передач у журналі SQLite WAL.",
            .refreshButton: "Оновити",
            .searchJobOrPath: "Пошук ID або шляху...",
            .maskPersonalPaths: "Маскувати особисті шляхи",
            .noHistoryFound: "Історію завдань не знайдено"
        ]
    ]
}
