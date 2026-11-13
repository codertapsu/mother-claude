import { HelpTranslation } from '../help-localized';
import { VI_TOPICS_A } from './topics-a';
import { VI_TOPICS_B } from './topics-b';
import { VI_TOPICS_C } from './topics-c';

/** The Help in Vietnamese (docs/I18N.md "Help"). */
export const VI_HELP: HelpTranslation = {
  groups: {
    basics: 'Cơ bản',
    sessions: 'Phiên của bạn',
    teams: 'Nhóm',
    machines: 'Máy tính của bạn',
    data: 'Tài khoản và dữ liệu',
    settings: 'Cài đặt và kết nối',
    troubleshooting: 'Khắc phục sự cố',
  },
  topics: { ...VI_TOPICS_A, ...VI_TOPICS_B, ...VI_TOPICS_C },
};
