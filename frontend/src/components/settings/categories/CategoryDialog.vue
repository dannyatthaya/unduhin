<script setup lang="ts">
// New category / Edit category.
//
// An extension belongs to one category. Adding one that another category
// already has asks whether to move it; Save stays off until every such
// extension is either moved or dropped, so there is never a clash left to
// sort out afterwards.
//
// Enter does not submit here: in the extensions box it adds what was typed.

import { computed, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { AlertCircle, AlertTriangle, CheckCircle2, Folder, Trash2 } from "lucide-vue-next";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import Field from "@/components/ui/Field.vue";
import Switch from "@/components/ui/Switch.vue";

import ChipInput from "@/components/settings/controls/ChipInput.vue";
import CategoryIconPicker from "@/components/settings/categories/CategoryIconPicker.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useGeneralSettings } from "@/composables/useGeneralSettings";
import { colorClass, colorFor, iconFor } from "@/lib/categoryIcons";
import {
  isCategoryNameTaken,
  isFallbackCategory,
  ownerOfExtension,
} from "@/lib/categoryMatch";
import type {
  Category,
  CategoryColor,
  NewCategoryInput,
} from "@/types/tauri-bindings";

const DEFAULT_ICON = "document";
const DEFAULT_COLOR: CategoryColor = "blue";

const { t } = useI18n();

const props = defineProps<{
  open: boolean;
  category?: Category | null;
}>();
const emit = defineEmits<{ close: []; delete: [category: Category] }>();

const store = useCategoriesStore();
const downloads = useDownloadsStore();
const generalSettings = useGeneralSettings();

const nameId = useId();
const nameErrorId = useId();
const previewId = useId();
const iconLabelId = useId();
const colorLabelId = useId();
const folderId = useId();
const ruleId = useId();
const chipHintId = useId();

const name = ref("");
const nameTouched = ref(false);
const icon = ref(DEFAULT_ICON);
const color = ref<CategoryColor>(DEFAULT_COLOR);
const folder = ref("");
const wantsRule = ref(true);
const extensions = ref<string[]>([]);
/** Extensions the user chose to take from another category. */
const moved = ref<string[]>([]);
const saving = ref(false);
const error = ref<string | null>(null);

const editing = computed(() => props.category ?? null);
const isEdit = computed(() => editing.value != null);
const isFallback = computed(
  () => editing.value != null && isFallbackCategory(editing.value),
);

watch(
  () => [props.open, props.category],
  () => {
    if (!props.open) return;
    error.value = null;
    saving.value = false;
    nameTouched.value = false;
    moved.value = [];
    const cat = props.category;
    name.value = cat?.name ?? "";
    icon.value = cat?.icon ?? DEFAULT_ICON;
    color.value = cat ? colorFor(cat) : DEFAULT_COLOR;
    folder.value = cat?.default_output_path ?? "";
    extensions.value = cat ? [...cat.extension_rules] : [];
    wantsRule.value = cat ? cat.extension_rules.length > 0 : true;
  },
  { immediate: true },
);

// ── Name ────────────────────────────────────────────────────────────────

const trimmedName = computed(() => name.value.trim());
const nameTaken = computed(() =>
  isCategoryNameTaken(trimmedName.value, store.list, editing.value?.id ?? null),
);
const nameError = computed(() => {
  if (nameTaken.value) {
    const existing = store.list.find(
      (c) => c.name.trim().toLowerCase() === trimmedName.value.toLowerCase(),
    );
    return t("settings.categoriesNameTaken", { name: existing?.name ?? trimmedName.value });
  }
  if (trimmedName.value === "" && nameTouched.value) {
    return t("settings.categoriesNameRequired");
  }
  return "";
});

// ── Folder ──────────────────────────────────────────────────────────────

/** A folder named after the category, inside the download folder. */
const autoFolder = computed(() => {
  const base = generalSettings.defaultOutputPath.value;
  if (!base || !trimmedName.value) return "";
  const separator = base.includes("\\") ? "\\" : "/";
  return `${base.replace(/[\\/]+$/, "")}${separator}${trimmedName.value}`;
});
const isAutoFolder = computed(() => folder.value.trim() === "");
const saveTo = computed({
  get: () => folder.value || autoFolder.value,
  set: (value: string) => {
    folder.value = value === autoFolder.value ? "" : value;
  },
});

async function pickFolder() {
  const picked = await openFolderDialog({
    directory: true,
    multiple: false,
    defaultPath: saveTo.value || undefined,
  });
  if (typeof picked === "string") folder.value = picked;
}

// ── Extensions ──────────────────────────────────────────────────────────

interface Clash {
  extension: string;
  owner: Category;
}

/** Extensions another category has, not yet moved here or dropped. */
const clashes = computed<Clash[]>(() => {
  if (!wantsRule.value) return [];
  const exceptId = editing.value?.id ?? null;
  return extensions.value.flatMap((extension) => {
    if (moved.value.includes(extension)) return [];
    const owner = ownerOfExtension(extension, store.list, exceptId);
    return owner ? [{ extension, owner }] : [];
  });
});
const flagged = computed(() => clashes.value.map((c) => c.extension));

function moveHere(extension: string) {
  moved.value = [...moved.value, extension];
}

function keepThere(extension: string) {
  extensions.value = extensions.value.filter((e) => e !== extension);
}

// A moved extension that was then removed from the box is no longer moved.
watch(extensions, (list) => {
  moved.value = moved.value.filter((e) => list.includes(e));
});

// ── Preview, counts ─────────────────────────────────────────────────────

const downloadCount = computed(() =>
  editing.value
    ? downloads.all.filter((d) => d.category_id === editing.value?.id).length
    : 0,
);
const previewIcon = computed(() => iconFor(icon.value).icon);

const subtitle = computed(() =>
  editing.value
    ? t(
        "settings.categoriesEditSubtitle",
        { n: downloadCount.value, name: editing.value.name },
        downloadCount.value,
      )
    : t("settings.categoriesDialogHint"),
);

// ── Save ────────────────────────────────────────────────────────────────

const canSave = computed(
  () =>
    !saving.value &&
    trimmedName.value !== "" &&
    !nameTaken.value &&
    clashes.value.length === 0,
);

/** Take each moved extension away from the category that had it. */
async function releaseMovedExtensions() {
  const exceptId = editing.value?.id ?? null;
  const byOwner = new Map<number, { owner: Category; extensions: string[] }>();
  for (const extension of moved.value) {
    const owner = ownerOfExtension(extension, store.list, exceptId);
    if (!owner) continue;
    const entry = byOwner.get(owner.id) ?? { owner, extensions: [] };
    byOwner.set(owner.id, { owner, extensions: [...entry.extensions, extension] });
  }
  for (const { owner, extensions: taken } of byOwner.values()) {
    await store.update(owner.id, {
      name: owner.name,
      icon: owner.icon,
      default_output_path: owner.default_output_path,
      color: owner.color,
      extension_rules: owner.extension_rules.filter((e) => !taken.includes(e)),
    });
  }
}

async function submit() {
  nameTouched.value = true;
  if (!canSave.value) return;
  const input: NewCategoryInput = {
    name: trimmedName.value,
    icon: icon.value,
    color: color.value,
    // A new category gets a folder named after it; an edited one keeps
    // whatever is in the field, empty included.
    default_output_path:
      folder.value.trim() || (isEdit.value ? null : autoFolder.value || null),
    extension_rules: wantsRule.value && !isFallback.value ? extensions.value : [],
  };
  saving.value = true;
  error.value = null;
  try {
    if (wantsRule.value) await releaseMovedExtensions();
    if (editing.value) await store.update(editing.value.id, input);
    else await store.add(input);
    emit("close");
  } catch (e: unknown) {
    error.value = (e as { message?: string })?.message ?? String(e);
  } finally {
    saving.value = false;
  }
}

const LABEL = "text-xs font-medium text-dl-soft";
const HELP = "text-xs leading-[1.45] text-muted-foreground";
</script>

<template>
  <Dialog
    :open="open"
    :title="isEdit ? t('settings.categoriesEdit') : t('settings.categoriesNew')"
    :subtitle="subtitle"
    @close="emit('close')"
  >
    <div class="flex min-w-0 flex-col gap-1.5">
      <div class="flex items-end gap-4">
        <div class="flex min-w-0 flex-1 flex-col gap-1.5">
          <label :for="nameId" :class="LABEL">{{ t("settings.categoriesNameLabel") }}</label>
          <Field
            :id="nameId"
            v-model="name"
            type="text"
            :disabled="isFallback"
            :invalid="nameError !== ''"
            :aria-describedby="nameError ? nameErrorId : undefined"
            @blur="nameTouched = true"
          />
        </div>
        <div class="flex shrink-0 flex-col gap-1.5">
          <span :id="previewId" :class="LABEL">{{ t("settings.categoriesPreview") }}</span>
          <div
            role="img"
            :aria-labelledby="previewId"
            class="flex h-9 w-[180px] items-center gap-2.5 rounded-[9px] border border-dl-line bg-dl-chrome px-2.5"
          >
            <span
              class="flex h-6 w-6 shrink-0 items-center justify-center rounded-[7px]"
              :class="colorClass(color)"
            >
              <component :is="previewIcon" class="h-[15px] w-[15px]" aria-hidden="true" />
            </span>
            <span class="min-w-0 flex-1 truncate font-medium">
              {{ trimmedName || t("settings.categoriesPreviewEmpty") }}
            </span>
            <span class="shrink-0 font-mono text-xs text-muted-foreground">
              {{ downloadCount }}
            </span>
          </div>
        </div>
      </div>
      <span
        v-if="nameError"
        :id="nameErrorId"
        role="alert"
        class="flex items-start gap-1.5 text-xs leading-[1.45] text-danger"
      >
        <AlertCircle class="mt-0.5 h-[13px] w-[13px] shrink-0" aria-hidden="true" />
        {{ nameError }}
      </span>
      <span v-else-if="isFallback" :class="HELP">
        {{ t("settings.categoriesFallbackNameHint") }}
      </span>
    </div>

    <CategoryIconPicker
      v-model:icon="icon"
      v-model:color="color"
      :icon-label-id="iconLabelId"
      :color-label-id="colorLabelId"
    />

    <div class="flex min-w-0 flex-col gap-1.5">
      <label :for="folderId" :class="LABEL">{{ t("settings.categoriesFolderLabel") }}</label>
      <div class="flex gap-2">
        <Field
          :id="folderId"
          v-model="saveTo"
          mono
          type="text"
          spellcheck="false"
          class="flex-1"
          :placeholder="t('settings.categoriesFolderPlaceholder')"
        >
          <template #leading>
            <Folder class="h-[15px] w-[15px] shrink-0 text-muted-foreground" aria-hidden="true" />
          </template>
          <template v-if="isAutoFolder && saveTo && !isEdit" #trailing>
            <span
              class="flex h-[22px] shrink-0 items-center rounded-md bg-muted px-2 text-xs font-medium text-dl-soft"
            >
              {{ t("addUrl.autoTag") }}
            </span>
          </template>
        </Field>
        <Button @click="pickFolder">{{ t("common.browse") }}</Button>
      </div>
      <span :class="HELP">
        {{
          isEdit
            ? t("settings.categoriesFolderEditHint", { n: downloadCount }, downloadCount)
            : t("settings.categoriesFolderNewHint")
        }}
      </span>
    </div>

    <div
      v-if="!isFallback"
      class="flex flex-col gap-3 rounded-xl border border-dl-line bg-dl-group p-3.5"
    >
      <div class="flex items-center justify-between gap-4">
        <span class="flex flex-col gap-0.5">
          <span :id="ruleId" class="font-semibold">{{ t("settings.categoriesFileByExtension") }}</span>
          <span :class="HELP">
            {{
              wantsRule
                ? t("settings.categoriesFileByExtensionOn")
                : t("settings.categoriesFileByExtensionOff")
            }}
          </span>
        </span>
        <Switch v-model="wantsRule" :labelledby="ruleId" />
      </div>

      <template v-if="wantsRule">
        <ChipInput
          v-model="extensions"
          prefix="."
          :flagged="flagged"
          :placeholder="t('settings.categoriesAddExtension')"
          :input-label="t('settings.categoriesAddExtensionAria')"
          :describedby="chipHintId"
        />

        <div
          v-for="clash in clashes"
          :key="clash.extension"
          role="alert"
          class="flex flex-col gap-2.5 rounded-[10px] bg-dl-warn-soft px-3 py-2.5 text-xs leading-[1.45] shadow-[inset_0_0_0_1px_hsl(var(--warning))]"
        >
          <span class="flex items-start gap-2.5">
            <AlertTriangle class="mt-px h-[15px] w-[15px] shrink-0 text-warning" aria-hidden="true" />
            <span>
              {{
                t("settings.categoriesClash", {
                  extension: `.${clash.extension}`,
                  owner: clash.owner.name,
                })
              }}
            </span>
          </span>
          <span class="flex gap-2 pl-[25px]">
            <Button size="sm" @click="moveHere(clash.extension)">
              {{
                t("settings.categoriesClashMove", {
                  extension: `.${clash.extension}`,
                  name: trimmedName || t("settings.categoriesClashHere"),
                })
              }}
            </Button>
            <Button size="sm" variant="ghost" @click="keepThere(clash.extension)">
              {{ t("settings.categoriesClashKeep", { owner: clash.owner.name }) }}
            </Button>
          </span>
        </div>

        <div class="flex justify-between gap-3">
          <span
            v-if="extensions.length > 0 && clashes.length === 0"
            class="flex items-center gap-1.5 text-xs text-success"
          >
            <CheckCircle2 class="h-[13px] w-[13px]" aria-hidden="true" />
            {{ t("settings.categoriesNoClash") }}
          </span>
          <span v-else />
          <span :id="chipHintId" :class="HELP">{{ t("settings.categoriesChipHint") }}</span>
        </div>
      </template>
    </div>

    <p v-if="error" role="alert" class="text-xs leading-[1.45] text-danger">{{ error }}</p>

    <template #footer>
      <Button
        v-if="editing && !isFallback"
        variant="dangerGhost"
        class="mr-auto px-2.5"
        :disabled="saving"
        @click="emit('delete', editing)"
      >
        <Trash2 class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ t("settings.categoriesDeleteEllipsis") }}
      </Button>
      <Button :disabled="saving" @click="emit('close')">
        {{ t("common.cancel") }}
      </Button>
      <Button variant="primary" :disabled="!canSave" @click="submit">
        {{ isEdit ? t("settings.categoriesSaveChanges") : t("settings.categoriesCreate") }}
      </Button>
    </template>
  </Dialog>
</template>
