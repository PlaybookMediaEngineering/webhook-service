<template>
  <form @submit="upsert" ref="form">
    <hook0-card>
      <hook0-card-header>
        <template #header v-if="isNew">
          Create new organization
        </template>
        <template #header v-else>
          Edit organization
        </template>
        <template #subtitle>
          An organization holds your team members
        </template>

      </hook0-card-header>
      <hook0-card-content>
        <hook0-card-content-line>
          <template #label>
            Organization Name
          </template>
          <template #content>
            <hook0-input
              type="text"
              v-model="organization.name"
              placeholder="my awesome api - production"
              required
            >
              <template #helpText></template>
            </hook0-input>
          </template>
        </hook0-card-content-line>
      </hook0-card-content>

      <hook0-card-content v-if="alert.visible">
        <hook0-alert :type="alert.type" :title="alert.title" :description="alert.description"></hook0-alert>
      </hook0-card-content>
      <hook0-card-footer>
        <hook0-button class="secondary" type="button" @click="$router.back()">Cancel</hook0-button>
        <hook0-button class="primary" type="button" @click="upsert($event)">{{
            isNew ? 'Create' : 'Update'
          }}
        </hook0-button>
      </hook0-card-footer>
    </hook0-card>
  </form>

</template>

<script lang="ts">
import {AxiosError} from 'axios';
import * as OrganizationService from './OrganizationService';
import {Organization} from './OrganizationService';
import {Options, Vue} from 'vue-class-component';
import {routes} from "@/routes";
import Hook0Alert, {AlertStatus} from "@/components/Hook0Alert.vue";

import {definitions} from '@/types';
import {UUID} from "@/http";

export type Problem = definitions['Problem'];

// For some reason typescript-eslint considers that Alert is of type any if it is imported from components/Hook0Alert.vue
interface Alert {
  title: string,
  description: string,
  type: AlertStatus,
  visible: boolean,
}

@Options({
  components: {
    Hook0Alert
  },
})
export default class OrganizationEdit extends Vue {
  private isNew = true;

  organization_id: UUID | undefined;

  routes = routes;

  organization = {
    name: '',
  };

  alert: Alert = {
    visible: false,
    type: 'alert',
    title: '',
    description: '',
  };

  mounted() {
    this.organization_id = this.$route.params.id as UUID;
    this.isNew = !this.organization_id;

    if (!this.isNew) {
      OrganizationService.get(this.organization_id).then((organization: Organization) => {
        this.organization.name = organization.name;
      }).catch(this.displayError.bind(this));
    }
  }

  upsert(e: Event) {
    e.preventDefault();
    e.stopImmediatePropagation();

    this.alert.visible = false; // reset alert

    if (this.isNew) {
      OrganizationService.create({
        name: this.organization.name,
      }).then(async (_resp: any) => {
        await this.$router.push({
          name: routes.OrganizationsList,
        });
      }, this.displayError.bind(this))
      return;
    }

    OrganizationService.update(this.$route.query.organization_id as string, {
      name: this.organization.name,
    }).then(async (_resp: any) => {
      await this.$router.push({
        name: routes.OrganizationsList,
      });
    }, this.displayError.bind(this))
  }

  displayError(err: AxiosError | unknown) {
    console.error(err);
    this.alert.visible = true;

    if (isAxiosError(err) && err.response) {
      const problem: Problem = err.response.data as Problem;
      this.alert.type = problem.status >= 500 ? 'alert' : 'warning';
      this.alert.title = problem.title;
      this.alert.description = problem.detail;
    } else {
      this.alert.type = 'alert';
      this.alert.title = "An error occurred";
      this.alert.description = String(err);
    }
  }
};

function isAxiosError(err: AxiosError | unknown): err is AxiosError {
  const e = err as AxiosError;
  return e !== null && typeof e.isAxiosError === 'boolean' && e.isAxiosError
}
</script>

<style scoped>
</style>