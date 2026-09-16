import AppKit
import SwiftUI

// MARK: - Flow Layout (wrapping action rows)
//
// Cards are 340–460pt wide but hold 5–6 action buttons (~550pt intrinsic).
// A plain HStack compresses children below intrinsic width, and SwiftUI Text
// then wraps character-per-line (vertical glyph soup). FlowLayout wraps
// buttons onto a second line at intrinsic size instead — compression never
// happens, so labels always render horizontally.

struct FlowLayout: Layout {
    var spacing: CGFloat = 8
    var lineSpacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let maxWidth = proposal.width ?? .infinity
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + spacing + size.width > maxWidth {
                x = 0
                y += rowHeight + lineSpacing
                rowHeight = 0
            }
            if x > 0 { x += spacing }
            x += size.width
            rowHeight = max(rowHeight, size.height)
        }
        return CGSize(width: proposal.width ?? x, height: y + rowHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX
        var y = bounds.minY
        var rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > bounds.minX, x + spacing + size.width > bounds.maxX {
                x = bounds.minX
                y += rowHeight + lineSpacing
                rowHeight = 0
            }
            if x > bounds.minX { x += spacing }
            subview.place(at: CGPoint(x: x, y: y), proposal: .unspecified)
            x += size.width
            rowHeight = max(rowHeight, size.height)
        }
    }
}

// MARK: - Model Hub View (LM Studio style Hugging Face model discovery)

public struct ModelHubView: View {
    @StateObject private var hub = ModelHubStore()
    @EnvironmentObject private var chatStore: ChatStore
    @EnvironmentObject private var network: NetworkManager
    @Binding var sidebarVisibility: NavigationSplitViewVisibility
    public var onSelectModel: ((String) -> Void)?

    public init(
        sidebarVisibility: Binding<NavigationSplitViewVisibility> = .constant(.all),
        onSelectModel: ((String) -> Void)? = nil
    ) {
        self._sidebarVisibility = sidebarVisibility
        self.onSelectModel = onSelectModel
    }

    public var body: some View {
        VStack(spacing: 0) {
            headerBar
            Divider().opacity(0.3)

            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    if hub.hubTab == .discover {
                        searchAndFiltersSection
                        searchErrorBanner
                        modelsGrid
                    } else {
                        installedModelsView
                    }
                }
                .padding(24)
            }
        }
        .background(VisualEffectView().ignoresSafeArea())
        .sheet(item: $hub.inspectingModel) { item in
            repoInspectorSheet(item)
        }
    }

    // MARK: Header Bar

    private var headerBar: some View {
        HStack(spacing: 12) {
            Button {
                withAnimation(LiquidGlass.spring) {
                    sidebarVisibility = (sidebarVisibility == .detailOnly) ? .all : .detailOnly
                }
                LiquidGlass.haptic(.alignment)
            } label: {
                Image(systemName: "sidebar.leading")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundColor(.secondary)
                    .frame(width: 28, height: 28)
                    .background(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .fill(Color.white.opacity(0.06))
                    )
            }
            .buttonStyle(.plain)
            .help("Toggle Sidebar (⌘B)")

            VStack(alignment: .leading, spacing: 2) {
                Text("Model Hub & Library")
                    .font(.system(size: 13, weight: .bold))
                Text(hub.hubTab == .discover ? "Hugging Face Discovery · LM Studio Style" : "On-Device Storage · Apple Silicon Weights")
                    .font(.system(size: 10))
                    .foregroundColor(.secondary)
            }

            Spacer()

            // Segmented control: label hidden (the header title already says
            // what this is) so the "Hub Mode" text is never squeezed into a
            // ~10pt column and stacked vertically. Short titles keep the
            // control inside its 300pt budget without truncating.
            Picker("", selection: $hub.hubTab) {
                ForEach(ModelHubStore.HubTab.allCases) { tab in
                    Text(tab.shortLabel).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 300)

            Spacer()

            if hub.isLoading || hub.isScanningInstalled {
                ProgressView().scaleEffect(0.7)
            }

            Button {
                LiquidGlass.haptic(.alignment)
                if hub.hubTab == .discover {
                    Task { await hub.fetchTopModels() }
                } else {
                    hub.scanInstalledModels()
                }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11))
            }
            .controlSize(.small)
            .lacGlass()
            .help("Refresh models")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
    }

    // MARK: Search & Filter Tabs

    private var searchAndFiltersSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            // Search Bar
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .foregroundColor(.secondary)
                    .font(.system(size: 13))

                TextField("Search 100,000+ models on Hugging Face (e.g. Qwen 3.8, Coder 32B, DeepSeek)...", text: $hub.searchQuery)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
                    .onChange(of: hub.searchQuery) { _ in
                        hub.onQueryChanged()
                    }

                if !hub.searchQuery.isEmpty {
                    Button {
                        hub.searchQuery = ""
                        hub.onQueryChanged()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundColor(.secondary)
                            .font(.system(size: 12))
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(.ultraThinMaterial)
                    .overlay(LiquidGlass.specularBorder(cornerRadius: 12))
            )

            // Filter Tabs
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(ModelHubStore.ModelFilter.allCases) { filter in
                        let isSelected = hub.selectedFilter == filter
                        Button {
                            LiquidGlass.haptic(.alignment)
                            hub.setFilter(filter)
                        } label: {
                            Text(filter.rawValue)
                                .font(.system(size: 11.5, weight: isSelected ? .semibold : .regular))
                                .padding(.horizontal, 12)
                                .padding(.vertical, 6)
                                .background(
                                    Capsule(style: .continuous)
                                        .fill(isSelected ? Color.accentColor : Color.white.opacity(0.08))
                                )
                                .foregroundColor(isSelected ? .white : .primary)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            // Model Size Filter Row (LM Studio style parameter categorization).
            // Flow: same compression-proofing as the card action rows — on
            // narrow windows capsules wrap instead of stacking vertically.
            FlowLayout(spacing: 8, lineSpacing: 8) {
                Text("Param Size:")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(.secondary)

                ForEach(ModelHubStore.ModelSizeFilter.allCases) { sizeFilter in
                    let isSelected = hub.selectedSize == sizeFilter
                    Button {
                        LiquidGlass.haptic(.alignment)
                        hub.selectedSize = sizeFilter
                    } label: {
                        Text(sizeFilter.rawValue)
                            .font(.system(size: 10.5, weight: isSelected ? .semibold : .regular))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(
                                Capsule(style: .continuous)
                                    .fill(isSelected ? Color.indigo : Color.white.opacity(0.06))
                            )
                            .foregroundColor(isSelected ? .white : .secondary)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    // MARK: Search Error Banner

    @ViewBuilder
    private var searchErrorBanner: some View {
        if let err = hub.errorMessage {
            HStack(spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundColor(.orange)
                    .font(.system(size: 12))
                Text(err)
                    .font(.system(size: 12))
                    .foregroundColor(.secondary)
                Spacer()
                Button("Retry") {
                    Task { await hub.fetchTopModels() }
                }
                .controlSize(.small)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.orange.opacity(0.10))
            )
        }
    }

    // MARK: Models Grid

    private var modelsGrid: some View {
        VStack(spacing: 16) {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 340, maximum: 460), spacing: 16)], spacing: 16) {
                ForEach(hub.filteredModels) { item in
                    ModelCard(
                        item: item,
                        expectedBytes: hub.estimatedBytes(for: item),
                        onSelect: {
                            chatStore.selectedModel = item.id
                            LiquidGlass.haptic(.alignment)
                            onSelectModel?(item.id)
                        },
                        onInspect: {
                            hub.inspectModelRepo(item)
                        }
                    )
                }
            }
            if hub.hasMoreResults {
                Button {
                    LiquidGlass.haptic(.alignment)
                    hub.loadMore()
                } label: {
                    HStack(spacing: 6) {
                        if hub.isLoading {
                            ProgressView().scaleEffect(0.7)
                        } else {
                            Image(systemName: "plus.circle")
                        }
                        Text(hub.isLoading ? "Loading…" : "Load more models (\(hub.filteredModels.count) shown)")
                    }
                }
                .lacGlass()
                .disabled(hub.isLoading)
                .padding(.top, 4)
            }
        }
    }

    // MARK: Installed Models View (On-Device Storage & Local Library)

    private var totalInstalledFormatted: String {
        let gb = Double(hub.totalInstalledBytes) / (1024.0 * 1024.0 * 1024.0)
        if gb >= 1.0 {
            return String(format: "%.1f GB", gb)
        }
        let mb = Double(hub.totalInstalledBytes) / (1024.0 * 1024.0)
        return String(format: "%.0f MB", mb)
    }

    private var freeDiskFormatted: String {
        let gb = Double(hub.freeDiskBytes) / (1024.0 * 1024.0 * 1024.0)
        return String(format: "%.1f GB free", gb)
    }

    private var storageTelemetryBanner: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        Image(systemName: "internaldrive.fill")
                            .font(.system(size: 13))
                            .foregroundColor(.accentColor)
                        Text("On-Device Storage Telemetry")
                            .font(.system(size: 13, weight: .bold))
                    }
                    Text("Live scan of /Volumes/AIModels, ~/.lac/models, ~/.cache/huggingface, and ~/.ollama")
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                }

                Spacer()

                HStack(spacing: 12) {
                    VStack(alignment: .trailing, spacing: 2) {
                        Text(totalInstalledFormatted)
                            .font(.system(size: 13, weight: .bold, design: .monospaced))
                            .foregroundColor(.primary)
                        Text("\(hub.installedModels.count) models stored")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }

                    Divider().frame(height: 24).opacity(0.3)

                    VStack(alignment: .trailing, spacing: 2) {
                        Text(freeDiskFormatted)
                            .font(.system(size: 13, weight: .bold, design: .monospaced))
                            .foregroundColor(.green)
                        Text("Mac SSD Available")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                }
            }

            // Dual capacity bar
            let total = Double(hub.totalInstalledBytes + hub.freeDiskBytes)
            let usedRatio = total > 0 ? min(1.0, Double(hub.totalInstalledBytes) / total) : 0.05
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(Color.white.opacity(0.10))
                    Capsule()
                        .fill(
                            LinearGradient(
                                colors: [Color.accentColor, Color.purple],
                                startPoint: .leading,
                                endPoint: .trailing
                            )
                        )
                        .frame(width: max(4, geo.size.width * CGFloat(usedRatio)))
                }
            }
            .frame(height: 6)
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: false)
    }

    private var installedModelsView: some View {
        VStack(alignment: .leading, spacing: 18) {
            storageTelemetryBanner

            if hub.installedModels.isEmpty {
                VStack(spacing: 14) {
                    Image(systemName: "square.stack.3d.up.slash")
                        .font(.system(size: 36))
                        .foregroundColor(.secondary.opacity(0.6))
                    Text("No On-Device Models Detected")
                        .font(.system(size: 15, weight: .semibold))
                    Text("Place MLX weights in `~/.lac/models` or an external drive at `/Volumes/AIModels` (recommended per AGENTS.md), or download directly from Hugging Face.")
                        .font(.system(size: 12))
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 480)

                    Button {
                        LiquidGlass.haptic(.alignment)
                        hub.hubTab = .discover
                    } label: {
                        HStack(spacing: 5) {
                            Image(systemName: "magnifyingglass")
                            Text("Browse Hugging Face Hub")
                        }
                    }
                    .controlSize(.small)
                    .lacGlassProminent()
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 40)
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 340, maximum: 460), spacing: 16)], spacing: 16) {
                    ForEach(hub.installedModels) { item in
                        InstalledModelCard(
                            item: item,
                            isSelected: chatStore.selectedModel == item.id || chatStore.selectedModel == item.name,
                            onSelect: {
                                chatStore.selectedModel = item.id
                                LiquidGlass.haptic(.alignment)
                                onSelectModel?(item.id)
                            },
                            onReveal: {
                                hub.revealInFinder(item: item)
                            },
                            onDelete: {
                                hub.deleteInstalledModel(item: item)
                            }
                        )
                    }
                }
            }
        }
    }

    // MARK: - Hugging Face Repo File & Quantization Inspector Sheet (LM Studio style)

    private func repoInspectorSheet(_ item: HFModelItem) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            // Header
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Image(systemName: "doc.text.magnifyingglass")
                            .font(.system(size: 14, weight: .bold))
                            .foregroundColor(.accentColor)
                        Text(item.modelName)
                            .font(.system(size: 14, weight: .bold))
                    }
                    Text("Repository: \(item.id) · LM Studio Quantization Inspector")
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                }

                Spacer()

                Button {
                    LiquidGlass.haptic(.alignment)
                    hub.inspectingModel = nil
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                        .font(.system(size: 14))
                }
                .buttonStyle(.plain)
            }

            Divider().opacity(0.2)

            if hub.isLoadingRepoFiles {
                VStack(spacing: 12) {
                    ProgressView()
                    Text("Querying Hugging Face file tree & quantizations...")
                        .font(.system(size: 11.5))
                        .foregroundColor(.secondary)
                }
                .frame(maxWidth: .infinity, minHeight: 180)
            } else if let err = hub.repoFilesError {
                VStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle")
                        .font(.system(size: 24))
                        .foregroundColor(.orange)
                    Text("Failed to inspect files: \(err)")
                        .font(.system(size: 12))
                        .foregroundColor(.secondary)
                    Button("Retry") {
                        hub.inspectModelRepo(item)
                    }
                    .lacGlass()
                }
                .frame(maxWidth: .infinity, minHeight: 180)
            } else if hub.repoFiles.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "doc.questionmark")
                        .font(.system(size: 24))
                        .foregroundColor(.secondary)
                    Text("No individual GGUF or SafeTensors files detected in root directory.")
                        .font(.system(size: 12))
                        .foregroundColor(.secondary)
                }
                .frame(maxWidth: .infinity, minHeight: 180)
            } else {
                Text("\(hub.repoFiles.count) file(s) available in repository:")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)

                ScrollView {
                    VStack(spacing: 8) {
                        ForEach(hub.repoFiles) { file in
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    HStack(spacing: 6) {
                                        Text(file.quantizationTag)
                                            .font(.system(size: 9.5, weight: .bold, design: .monospaced))
                                            .padding(.horizontal, 5)
                                            .padding(.vertical, 1.5)
                                            .background(Capsule().fill(Color.accentColor.opacity(0.18)))
                                            .foregroundColor(.accentColor)
                                        Text(file.fileName)
                                            .font(.system(size: 11.5, weight: .medium, design: .monospaced))
                                            .lineLimit(1)
                                    }
                                    Text("Size: \(file.formattedSize)")
                                        .font(.system(size: 10))
                                        .foregroundColor(.secondary)
                                }

                                Spacer()

                                Button {
                                    LiquidGlass.haptic(.alignment)
                                    // The backend pulls the whole repo, not the
                                    // single file: gate on the manifest total.
                                    let total = hub.repoFiles.compactMap(\.size).reduce(0, +)
                                    network.pullModel(item.id, expectedBytes: total > 0 ? total : nil)
                                    hub.inspectingModel = nil
                                } label: {
                                    HStack(spacing: 4) {
                                        Image(systemName: "arrow.down.circle")
                                        Text("Pull repo")
                                    }
                                    .font(.system(size: 10.5))
                                }
                                .help("Downloads the full repository (all quants), not just this file")
                                .controlSize(.small)
                                .lacGlass()
                            }
                            .padding(10)
                            .liquidGlassCard(cornerRadius: 10, hoverable: true)
                        }
                    }
                    .padding(.vertical, 2)
                }
                .frame(maxHeight: 280)
            }

            HStack {
                Button {
                    LiquidGlass.haptic(.alignment)
                    ModelHubStore.openInBrowser(modelId: item.id)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.up.forward.square")
                        Text("View on Hugging Face")
                    }
                    .font(.system(size: 11))
                }
                .lacGlass()

                Spacer()

                Button("Done") {
                    LiquidGlass.haptic(.alignment)
                    hub.inspectingModel = nil
                }
                .lacGlassProminent()
            }
        }
        .padding(20)
        .frame(width: 520)
        .liquidGlassModalCard(cornerRadius: 18)
    }
}

// MARK: - Model Card Component (LM Studio style)

struct ModelCard: View {
    let item: HFModelItem
    let expectedBytes: Int64?
    var onSelect: () -> Void
    var onInspect: (() -> Void)? = nil
    @EnvironmentObject private var network: NetworkManager
    @State private var copied = false
    @State private var copiedCommand = false
    @State private var isHovered = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            // Header: Author / Name and Quantization badge
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.author)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundColor(.secondary)
                    Text(item.modelName)
                        .font(.system(size: 13, weight: .bold))
                        .lineLimit(1)
                        .foregroundColor(.primary)
                }

                Spacer()

                HStack(spacing: 4) {
                    Text(item.quantization)
                        .font(.system(size: 9.5, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.accentColor.opacity(0.2)))
                        .foregroundColor(.accentColor)

                    if item.isMlx {
                        Text("MLX")
                            .font(.system(size: 9.5, weight: .bold))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Capsule().fill(Color.orange.opacity(0.2)))
                            .foregroundColor(.orange)
                    }
                }
            }

            // Stats row (Downloads, Likes, Pipeline)
            HStack(spacing: 12) {
                HStack(spacing: 4) {
                    Image(systemName: "arrow.down.circle.fill")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                    Text("\(item.downloadFormatted) downloads")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundColor(.secondary)
                }

                if let likes = item.likes, likes > 0 {
                    HStack(spacing: 3) {
                        Image(systemName: "heart.fill")
                            .font(.system(size: 9))
                            .foregroundColor(.pink.opacity(0.8))
                        Text("\(likes)")
                            .font(.system(size: 11))
                            .foregroundColor(.secondary)
                    }
                }

                Spacer()
            }

            // RAM Fit Indicator
            let fit = item.ramFitEstimate(hostRamGB: network.host?.total_ram_gib)
            HStack(spacing: 6) {
                Circle()
                    .fill(fit.color)
                    .frame(width: 6, height: 6)
                Text(fit.label)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(fit.color)
                Spacer()
            }
            .padding(.vertical, 2)

            Divider().opacity(0.2)

            // Actions row (flow: wraps to a second line at intrinsic size
            // instead of compressing labels into vertical glyph soup)
            FlowLayout(spacing: 8, lineSpacing: 8) {
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(item.id, forType: .string)
                    LiquidGlass.haptic(.alignment)
                    withAnimation(LiquidGlass.spring) { copied = true }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                        withAnimation(LiquidGlass.spring) { copied = false }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: copied ? "checkmark" : "doc.on.doc")
                            .font(.system(size: 10))
                        Text(copied ? "Copied" : "Copy ID")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()

                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString("lac pull \(item.id)", forType: .string)
                    LiquidGlass.haptic(.alignment)
                    withAnimation(LiquidGlass.spring) { copiedCommand = true }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                        withAnimation(LiquidGlass.spring) { copiedCommand = false }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: copiedCommand ? "checkmark" : "terminal")
                            .font(.system(size: 10))
                        Text(copiedCommand ? "Copied" : "Pull Cmd")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()
                Button {
                    LiquidGlass.haptic(.alignment)
                    // Fail-open by design when the manifest was never
                    // inspected (see estimatedBytes): use Files → Pull repo
                    // for a size-gated download.
                    network.pullModel(item.id, expectedBytes: expectedBytes)
                } label: {
                    HStack(spacing: 4) {
                        if network.pullingModelId == item.id {
                            ProgressView()
                                .controlSize(.mini)
                            Text("Pulling...")
                                .font(.system(size: 11))
                        } else {
                            Image(systemName: "arrow.down.circle")
                                .font(.system(size: 10))
                            Text("Pull")
                                .font(.system(size: 11))
                        }
                    }
                }
                .controlSize(.small)
                .lacGlass()
                .disabled(network.pullingModelId != nil && network.pullingModelId != item.id)
                if network.pullingModelId == item.id {
                    Button {
                        LiquidGlass.haptic(.alignment)
                        network.cancelPull()
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "xmark.circle")
                                .font(.system(size: 10))
                            Text("Cancel")
                                .font(.system(size: 11))
                        }
                    }
                    .controlSize(.small)
                    .lacGlass()
                }
                Button {
                    LiquidGlass.haptic(.alignment)
                    onInspect?()
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: "list.bullet.indent")
                            .font(.system(size: 10))
                        Text("Files")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()
                .help("Inspect available quantization files (GGUF, SafeTensors) in repo")

                Button {
                    LiquidGlass.haptic(.alignment)
                    ModelHubStore.openInBrowser(modelId: item.id)
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: "arrow.up.forward.square")
                            .font(.system(size: 10))
                        Text("HF")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()
                .help("Open model repository on Hugging Face")

                Button {
                    onSelect()
                } label: {
                    HStack(spacing: 5) {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 11))
                        Text("Use for Chat")
                            .font(.system(size: 11, weight: .semibold))
                    }
                }
                .controlSize(.small)
                .lacGlassProminent()
            }
            
            if network.pullingModelId == item.id, let output = network.pullOutput {
                Divider().opacity(0.15)
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text("PULL LOG")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundColor(.secondary)
                        Spacer()
                        if let progress = network.pullProgress {
                            Text("\(Int(progress * 100))%")
                                .font(.system(size: 9, design: .monospaced))
                                .foregroundColor(.secondary)
                        }
                    }
                    if let progress = network.pullProgress {
                        ProgressView(value: progress)
                            .controlSize(.mini)
                    }
                    
                    ScrollView {
                        Text(output)
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.primary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(height: 80)
                    .padding(8)
                    .background(Color.black.opacity(0.2))
                    .cornerRadius(8)
                }
            }
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: true)
    }
}

// MARK: - Installed Model Card (On-Device Storage)

struct InstalledModelCard: View {
    let item: InstalledModelItem
    let isSelected: Bool
    var onSelect: () -> Void
    var onReveal: () -> Void
    var onDelete: () -> Void
    @State private var showingDeleteConfirm = false
    @State private var copiedPath = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            // Title & Format
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.name)
                        .font(.system(size: 13, weight: .bold))
                        .lineLimit(1)
                        .foregroundColor(.primary)
                    Text(item.sourceDir)
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                        .lineLimit(1)
                }

                Spacer()

                HStack(spacing: 4) {
                    Text(item.format)
                        .font(.system(size: 9.5, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.accentColor.opacity(0.2)))
                        .foregroundColor(.accentColor)

                    Text(item.formattedSize)
                        .font(.system(size: 9.5, weight: .bold, design: .monospaced))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(Color.white.opacity(0.08)))
                        .foregroundColor(.primary)
                }
            }

            // RAM Fit Indicator
            let rec = item.ramRecommendation
            HStack(spacing: 6) {
                Circle()
                    .fill(rec.color)
                    .frame(width: 6, height: 6)
                Text(rec.label)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(rec.color)

                Spacer()

                if isSelected {
                    HStack(spacing: 4) {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 10))
                        Text("Active")
                            .font(.system(size: 10, weight: .semibold))
                    }
                    .foregroundColor(.green)
                }
            }

            Divider().opacity(0.2)

            // Action row (flow: wraps instead of compressing labels vertical)
            FlowLayout(spacing: 8, lineSpacing: 8) {
                Button {
                    onReveal()
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: "folder")
                            .font(.system(size: 10))
                        Text("Finder")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()
                .help("Reveal model files in macOS Finder")

                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(item.path, forType: .string)
                    LiquidGlass.haptic(.alignment)
                    withAnimation(LiquidGlass.spring) { copiedPath = true }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                        withAnimation(LiquidGlass.spring) { copiedPath = false }
                    }
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: copiedPath ? "checkmark" : "doc.on.doc")
                            .font(.system(size: 10))
                        Text(copiedPath ? "Copied" : "Path")
                            .font(.system(size: 11))
                    }
                }
                .controlSize(.small)
                .lacGlass()

                Button {
                    showingDeleteConfirm = true
                } label: {
                    Image(systemName: "trash")
                        .font(.system(size: 10))
                        .foregroundColor(.red.opacity(0.8))
                }
                .controlSize(.small)
                .lacGlass()
                .help("Delete local model files to reclaim disk space")
                .confirmationDialog(
                    "Delete \(item.name)?",
                    isPresented: $showingDeleteConfirm,
                    titleVisibility: .visible
                ) {
                    Button("Delete Model (\(item.formattedSize))", role: .destructive) {
                        onDelete()
                    }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text("This will permanently remove the model directory from disk: \(item.path)")
                }

                Button {
                    onSelect()
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: isSelected ? "checkmark" : "bolt.fill")
                            .font(.system(size: 10))
                        Text(isSelected ? "Active" : "Load Model")
                            .font(.system(size: 11, weight: .semibold))
                    }
                }
                .controlSize(.small)
                .lacGlassProminent()
            }
        }
        .padding(14)
        .liquidGlassCard(cornerRadius: 14, hoverable: true)
    }
}
