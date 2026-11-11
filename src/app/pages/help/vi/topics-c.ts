import { HelpTopicSource } from '../help-model';

/*
 * Vietnamese Help (docs/I18N.md "Help"): your computers (Machines), and
 * their troubleshooting. Each topic has the same sections in the same order
 * as its English one in `help-topics.ts`; section ids, `where`, embeds and
 * link paths come from the English, only the link labels from here.
 * `**bold**` names a label as the Vietnamese interface shows it
 * (`i18n/vi/machines.json`, `moves.json`); `…` stands for a computer's or an
 * account's name.
 */
const TOPICS: HelpTopicSource[] = [
  // ─────────────────────────────────────────────────── Máy tính của bạn
  {
    id: 'machines-setup',
    title: 'Thiết lập Các máy',
    icon: 'monitor',
    group: 'machines',
    summary:
      'Kết nối các máy tính của bạn: các mục kiểm tra trên từng máy, cách khắc phục, và bài kiểm tra trước khi bật.',
    intro:
      'Tính năng Các máy kết nối các máy tính của bạn: từ bất kỳ máy nào, bạn xem và điều khiển các phiên Claude Code của mọi máy, và chuyển một phiên, cùng mã nguồn của nó, sang máy tính khác rồi chuyển về. Tính năng này tắt cho đến khi bạn thiết lập nó trong ứng dụng máy tính. Việc thiết lập chạy trên từng máy, vì chỉ máy đó mới tự kiểm tra được chính nó.',
    keywords: [
      'các máy',
      'máy tính khác',
      'máy thứ hai',
      'thiết lập',
      'kiểm tra',
      'tường lửa',
      'mạng cục bộ',
      'máy chủ chuyển tiếp',
      'tình trạng',
      'đường dẫn dài',
    ],
    sections: [
      {
        heading: 'Bạn cần gì',
        points: [
          'Mother Claude, phiên bản này hoặc mới hơn, trên mỗi máy tính.',
          'Mỗi máy tính đều đang bật, có Internet hoặc ở cùng một mạng với các máy kia.',
          'Trên Windows, một lần đồng ý của quản trị viên, cho tường lửa.',
          'Khoảng 10 phút, khi có sẵn từng máy tính bên cạnh.',
        ],
      },
      {
        heading: 'Chạy thiết lập',
        where: 'desktop',
        intro:
          'Mở **Các máy** và nhấn **Thiết lập Các máy**. Có bảy bước: **Tiếp** và **Quay lại** để chuyển giữa các bước, và **Tiếp tục thiết lập** trên trang Các máy đưa bạn về bước bạn đã dừng.',
        steps: [
          '**Máy Mac này** (hoặc **Máy PC này**): sửa những gì các mục kiểm tra phát hiện. Các mục bắt buộc được xếp trước.',
          '**Các máy của bạn**: ghép nối từng máy tính khác của bạn.',
          '**Mọi máy**: các mục kiểm tra của từng máy, đặt cạnh nhau.',
          '**Quyền của từng máy**: những gì mỗi máy được làm trên máy này. Nhấn **Lưu** sau khi thay đổi.',
          '**Tài khoản và thư mục**: tài khoản và thư mục dùng chung của bạn trên mọi máy.',
          '**Tùy chọn**: máy chủ chuyển tiếp, cách chuyển mã nguồn của phiên và nơi mã nguồn được đặt.',
          '**Kiểm tra, rồi bật**: nhấn **Chạy kiểm tra**, rồi **Bật tính năng Các máy**.',
        ],
        tips: [
          '**Bật tính năng Các máy** bị làm mờ khi còn mục kiểm tra bắt buộc, và cho biết còn bao nhiêu mục. Cho đến khi bật, các máy tính của bạn chỉ ghép nối, chạy thiết lập và bài kiểm tra với nhau: không máy nào xem, điều khiển hay chuyển phiên của một máy đang tắt tính năng Các máy.',
          'Sau đó các mục kiểm tra vẫn tiếp tục chạy dưới tên **Tình trạng Các máy**: một chấm đỏ trên **Các máy** ở thanh bên báo có gì đó lại hỏng.',
        ],
        links: [
          { label: 'Mở trang Các máy', path: '/machines' },
          { label: 'Xem “Ghép nối các máy tính của bạn”', path: '/help/machines-pair' },
        ],
      },
      {
        heading: 'Những gì được kiểm tra',
        intro:
          'Mục **Bắt buộc** khiến tính năng Các máy chưa bật được; mục **Khuyến nghị** vẫn hoạt động, nhưng kém hơn. Mỗi mục cho biết vì sao nó quan trọng, và nút của nó sẽ sửa hoặc mở đúng trang cài đặt.',
        terms: [
          {
            term: 'Mother Claude',
            text: 'Mỗi máy tính cần một phiên bản dùng cùng giao thức Các máy. Hãy cập nhật máy cũ hơn.',
          },
          {
            term: 'Luôn thức khi đang được dùng',
            text: 'Máy tính đang ngủ không trả lời được các máy khác. **Giữ máy thức khi đang được dùng** giữ máy thức khi có máy tính đã ghép nối đang kết nối; không cài đặt hệ thống nào bị thay đổi.',
          },
          {
            term: 'Quyền truy cập Mạng cục bộ (máy Mac)',
            text: 'Không có quyền này, máy Mac chỉ tới được các máy tính khác của bạn qua máy chủ chuyển tiếp, kể cả khi ở nhà. Hãy cho phép khi macOS hỏi, hoặc nhấn **Mở cài đặt Mạng cục bộ** và bật Mother Claude.',
          },
          {
            term: 'Tường lửa (máy Mac)',
            text: 'Nếu tường lửa chặn Mother Claude, các máy khác chỉ tới được máy Mac qua máy chủ chuyển tiếp. Nhấn **Mở cài đặt Tường lửa** và cho phép Mother Claude.',
          },
          {
            term: 'Loại mạng (Windows)',
            text: 'Trên mạng Public, Windows chặn các kết nối đến. Nhấn **Mở cài đặt mạng** và chuyển mạng ở nhà hoặc văn phòng của bạn sang Private.',
          },
          {
            term: 'Tường lửa (Windows)',
            text: 'Không có quy tắc, các máy khác chỉ tới được máy PC qua máy chủ chuyển tiếp. **Cho phép qua tường lửa** thêm một quy tắc cho Mother Claude, sau khi Windows hỏi quản trị viên đồng ý.',
          },
          {
            term: 'Máy chủ chuyển tiếp',
            text: 'Khi khác mạng, các máy tính của bạn gặp nhau qua máy chủ chuyển tiếp. Hãy kiểm tra địa chỉ của nó, hoặc chọn máy chủ chuyển tiếp công cộng trong **Tùy chọn**.',
          },
          {
            term: 'Máy tính đã ghép nối',
            text: 'Tính năng Các máy hoạt động giữa các máy tính bạn đã ghép nối. Mỗi cặp máy cần cùng giao thức Các máy và cùng cài đặt máy chủ chuyển tiếp: **Dùng cài đặt này ở mọi nơi** đề xuất cài đặt của bạn cho các máy kia.',
          },
          {
            term: 'git',
            text: 'Mã nguồn của phiên được chuyển qua git, bản 2.30 hoặc mới hơn, trên cả hai máy tính: **Cài công cụ dòng lệnh** trên máy Mac, **Cài Git for Windows** trên Windows.',
          },
          {
            term: 'Đường dẫn dài (Windows)',
            text: 'Git for Windows bỏ qua các đường dẫn dài hơn 260 ký tự, trừ khi bật đường dẫn dài. **Bật cho git** bật nó cho bạn; cài đặt riêng của Windows cần quản trị viên.',
          },
          {
            term: 'Claude Code',
            text: 'Các phiên chạy trong Claude Code, và một phiên chuyển sang phiên bản cũ hơn có thể không tiếp tục được. **Cập nhật Claude Code** chạy `claude update` khi bạn nhấn nút.',
          },
          {
            term: 'Từng tài khoản',
            text: 'Đã đăng nhập trên máy tính này. Chỉ các phiên của tài khoản đó phải chờ: tính năng Các máy vẫn bật được khi thiếu nó. **Đăng nhập…** bắt đầu việc đăng nhập ngay tại máy này.',
          },
          {
            term: 'Dung lượng trống',
            text: 'Các lần chuyển được đặt vào thư mục dữ liệu của Mother Claude trước: nên còn trống 5 GB.',
          },
        ],
        notes: [
          'Các mục kiểm tra mạng chỉ bắt buộc khi chọn **Không dùng máy chủ chuyển tiếp**. Khi có máy chủ chuyển tiếp, chúng là khuyến nghị: các máy tính của bạn vẫn gặp nhau qua máy chủ đó, chỉ chậm hơn.',
        ],
      },
      {
        heading: 'Tài khoản và thư mục',
        points: [
          'Tài khoản được ghép theo tên trên từng máy tính, không phân biệt chữ hoa chữ thường, và tài khoản Mặc định với tài khoản Mặc định. Một phiên được chuyển sang tài khoản cùng tên, và chỉ chạy được ở nơi tài khoản đó đã đăng nhập.',
          'Máy khác chỉ thấy tên, loại và trạng thái đăng nhập của các tài khoản của bạn: không bao giờ thấy token, email hay thư mục.',
          'Thư mục dùng chung là một thư mục trên mỗi máy tính, chẳng hạn `~/development/projects` trên máy Mac và `D:\\development\\projects` trên Windows: một dự án nằm trong nó ở máy này là cùng dự án đó, ở cùng vị trí, trên máy kia.',
          'Phần thiết lập đề xuất thư mục dùng chung dựa trên remote git của các dự án: nhấn **Dùng** để nhận một thư mục, hoặc **Thêm thư mục** để thêm thư mục của riêng bạn. Các cặp thư mục từ **Sao lưu & đồng bộ** được giữ lại.',
          'Dự án chỉ một máy có sẽ hiện **Sao chép sang …** hoặc **Sao chép về đây**. Dự án không có remote git thì không thể chuyển cùng mã nguồn.',
          '**Đăng nhập trên …**, **Thêm trên …** và **Sao chép sang …** là những việc được nhờ máy đó: người ở máy đó sẽ xác nhận.',
        ],
      },
      {
        heading: 'Tùy chọn',
        terms: [
          {
            term: 'Máy chủ chuyển tiếp',
            text: '**Máy chủ chuyển tiếp công cộng (mặc định)** dùng được ngay, ở bất cứ đâu. **Máy chủ chuyển tiếp của riêng bạn** là một `iroh-relay` do bạn tự chạy, trên NAS hoặc một máy chủ nhỏ: nhập địa chỉ `https://` của nó. **Không dùng máy chủ chuyển tiếp**: các máy tính của bạn chỉ gặp nhau trong cùng một mạng. Máy chủ chuyển tiếp chỉ truyền dữ liệu đã mã hóa.',
          },
          {
            term: 'Cách mã nguồn được chuyển khi một phiên chuyển đi',
            text: '**Trực tiếp giữa các máy (mặc định)**: gửi thẳng sang máy tính kia, không đẩy lên đâu cả. **Qua remote git của bạn**: các ref ẩn trên remote của bạn, bị xoá sau khi chuyển trừ khi bạn giữ lại; cả hai máy cần truy cập được remote mà không phải nhập mật khẩu.',
          },
          {
            term: 'Nơi mã nguồn được đặt',
            text: '**Trong thư mục dự án, khi nó không có thay đổi riêng (mặc định)**, hoặc **Luôn ở một thư mục riêng (worktree), không bao giờ ở thư mục dự án**: một worktree trong thư mục dữ liệu của Mother Claude trên máy nhận phiên.',
          },
          {
            term: 'Giữ máy thức',
            text: 'Khi một máy tính khác đang dùng máy này, Mother Claude giữ máy thức. Không cài đặt hệ thống nào bị thay đổi.',
          },
        ],
        points: [
          'Hãy dùng cùng máy chủ chuyển tiếp trên mọi máy (hai máy dùng máy chủ chuyển tiếp khác nhau có thể không bao giờ gặp được nhau), và tốt nhất là cùng tùy chọn mã nguồn: thay đổi ở đây được đề xuất cho các máy kia, và mỗi máy nhận khi người ở máy đó nhấn **Dùng cả ở đây** (hoặc **Giữ cài đặt của máy này**). Một lần chuyển theo tùy chọn của máy bắt đầu chuyển.',
          '**Từng dự án** có thể chuyển mã nguồn theo cách khác, và có một công thức: **Sau khi chuyển, nhờ Claude chạy**, mỗi dòng một lệnh, cho macOS, Windows và Linux. Các lệnh được gửi cho Claude, và Claude chạy chúng qua các bước phê duyệt của chính phiên đó. **Các tệp git bỏ qua sẽ đi kèm**, chẳng hạn `.env.local`, đi cùng mỗi lần chuyển của dự án: chúng thường chứa bí mật, nên việc thêm tệp, và việc mang chúng theo, đều cần quyền thao tác nguy hiểm.',
          '**Kiểm tra remote** xem remote git của một dự án có phản hồi mà không yêu cầu mật khẩu trên từng máy không. Việc kiểm tra chỉ chạy khi bạn nhấn nút, và chạy lại trước mỗi lần chuyển qua remote.',
        ],
      },
    ],
  },
  {
    id: 'machines-pair',
    title: 'Ghép nối các máy tính của bạn',
    icon: 'key',
    group: 'machines',
    summary:
      'Ghép nối hai máy tính bằng một mã dùng một lần, và kiểm tra cùng sáu chữ số trên cả hai máy.',
    intro:
      'Hai máy tính chỉ cần ghép nối một lần, trong ứng dụng máy tính của mỗi máy: trong cùng mạng hoặc qua Internet, không cần mở cổng hay chạy đường hầm.',
    keywords: [
      'ghép nối',
      'tham gia',
      'mã',
      'ở gần',
      'sáu chữ số',
      'thu hồi',
      'hủy ghép nối',
      'đổi tên',
      'khóa máy',
    ],
    sections: [
      {
        heading: 'Ghép nối hai máy tính',
        where: 'desktop',
        steps: [
          'Trên máy tính thứ nhất, mở **Các máy** và nhấn **Ghép nối một máy**. Một mã 8 ký tự hiện ra: mã dùng được một lần, trong năm phút.',
          'Trên máy tính kia, mở **Các máy** và nhấn **Tham gia…**.',
          'Nhập mã vào **Mã** và nhấn **Tham gia**.',
          'Kiểm tra rằng cả hai màn hình hiện cùng sáu chữ số.',
          'Trên mỗi máy, chọn những gì máy kia được làm trên máy này, rồi nhấn **Dãy số khớp — Ghép nối**.',
        ],
        tips: [
          '**Ở gần** liệt kê các máy tính trong mạng của bạn đang hiện mã, để bạn kiểm tra máy nào đang hiện mã: bạn vẫn phải nhập mã của máy đó, vì việc tham gia cần mã. Không có máy nào ở gần? Mã dùng được cả qua Internet.',
          'Phần thiết lập có cùng các nút này ở bước **Các máy của bạn**.',
        ],
        links: [{ label: 'Mở trang Các máy', path: '/machines' }],
      },
      {
        heading: 'Qua Internet',
        points: [
          'Khi máy kia tham gia qua Internet, máy thứ nhất không hiện các chữ số: hãy nhập sáu chữ số mà máy kia hiển thị, rồi nhấn **Ghép nối**.',
          'Nhập sai một lần là lượt ghép nối kết thúc, và chưa có gì được ghép nối. Hãy bắt đầu lại với mã mới.',
        ],
      },
      {
        heading: 'Vì sao an toàn',
        points: [
          'Chỉ người có mã mới bắt đầu được việc ghép nối: mã giúp tìm máy kia, và chứng minh bạn đang giữ mã.',
          'Sáu chữ số được tính từ khóa của cả hai máy, nên một máy chen vào giữa, kể cả máy đã thấy mã, sẽ làm hai màn hình hiện hai dãy số khác nhau.',
          'Không có gì được lưu cho đến khi cả hai máy xác nhận.',
          'Máy đã ghép nối được nhận ra chỉ bằng khóa của nó, lưu trong Chuỗi khóa hoặc Windows Credential Manager. Cài lại Mother Claude sẽ tạo khóa mới: hãy ghép nối lại máy đó.',
        ],
      },
      {
        heading: 'Đổi tên hoặc thu hồi một máy',
        where: 'desktop',
        points: [
          'Trên **Các máy**, thẻ của mỗi máy đã ghép nối có **Đổi tên** (tên của nó trên máy này) và **Thu hồi**.',
          '**Thu hồi**, rồi **Nhấn lần nữa để thu hồi**: máy này quên máy kia và cắt các kết nối của nó ngay lập tức.',
          'Việc thu hồi chỉ có tác dụng trên một máy: hãy thu hồi một máy bị mất hoặc bị đánh cắp trên từng máy còn lại của bạn.',
          '**Tắt tính năng Các máy** không hủy ghép nối, nhưng khi tính năng này tắt, máy đã ghép nối chỉ ghép nối và chạy thiết lập được với máy này: nó không xem, điều khiển hay chuyển được các phiên ở đây, và máy này cũng không dùng các máy kia. **Thu hồi** để quên một máy.',
        ],
      },
    ],
  },
  {
    id: 'machines-permissions',
    title: 'Máy khác được làm gì',
    icon: 'shield',
    group: 'machines',
    summary: 'Các quyền mỗi máy tính cấp cho các máy khác, trong cùng mạng và qua Internet.',
    intro:
      'Mỗi máy tính tự quyết định từng máy khác được làm gì trên nó. Bạn chọn khi ghép nối, và có thể đổi sau trên trang **Các máy**.',
    keywords: [
      'quyền',
      'cấp quyền',
      'xem',
      'điều khiển',
      'di chuyển',
      'nguy hiểm',
      'nhật ký kiểm tra',
      'cùng mạng',
    ],
    sections: [
      {
        heading: 'Các quyền',
        terms: [
          {
            term: 'Xem',
            text: 'Liệt kê các phiên, đọc bản ghi, xem yêu cầu duyệt và câu hỏi. Bật theo mặc định.',
          },
          {
            term: 'Điều khiển',
            text: 'Gửi tin nhắn, trả lời yêu cầu duyệt thông thường, ngắt, dừng, và đặt mô hình, mức nỗ lực và chế độ. Bật theo mặc định.',
          },
          {
            term: 'Di chuyển',
            text: 'Chuyển phiên đến máy này và đi khỏi máy này. Bật theo mặc định.',
          },
          { term: 'Chạy', text: 'Chạy lệnh và tác vụ ở đây. Chưa có: quyền này luôn tắt.' },
          {
            term: 'Nguy hiểm',
            text: 'Bỏ qua quyền, cho phép các lệnh gọi công cụ bị đánh dấu nguy hiểm, gửi phím thô, và mang theo sự tin cậy của thư mục cùng các tệp git bỏ qua khi chuyển phiên. Tắt theo mặc định.',
          },
        ],
        points: [
          'Điều khiển cần quyền xem; di chuyển và thao tác nguy hiểm cần quyền điều khiển. Đánh dấu một quyền sẽ đánh dấu cả những quyền nó cần; bỏ một quyền sẽ bỏ cả những quyền cần nó.',
        ],
      },
      {
        heading: 'Cùng mạng hay qua Internet',
        points: [
          'Mỗi quyền có hai cột: **Cùng mạng**, cho kết nối trực tiếp trong mạng cục bộ của bạn, và **Internet**, cho mọi trường hợp khác, kể cả qua máy chủ chuyển tiếp.',
          'Thẻ của một máy trên **Các máy** cho biết hiện nó đang kết nối thế nào: **Trực tiếp (cùng mạng)**, **Trực tiếp (Internet)** hoặc **Qua máy chủ chuyển tiếp (chậm hơn)**.',
        ],
      },
      {
        heading: 'Thay đổi quyền',
        where: 'desktop',
        steps: [
          'Mở **Các máy**.',
          'Trên thẻ của máy kia, cạnh **Trên máy Mac này, … được** (hoặc **Trên máy PC này, … được**), nhấn **Thay đổi**.',
          'Đánh dấu hoặc bỏ đánh dấu các ô, rồi nhấn **Lưu**.',
        ],
        points: [
          '**Trên …, máy Mac này được** cho biết máy kia cho máy này làm gì. Chỉ máy kia mới thay đổi được.',
          'Phần thiết lập có cùng bảng này ở bước **Quyền của từng máy**.',
        ],
        links: [{ label: 'Mở trang Các máy', path: '/machines' }],
      },
      {
        heading: 'Thao tác nguy hiểm qua máy khác',
        points: [
          'Một thao tác nguy hiểm trên máy khác cần cả hai: quyền **Nguy hiểm** mà máy đó cấp cho máy này, và quyền của chính bạn ở đây. Một điện thoại không có quyền thao tác nguy hiểm trên máy này không thể dùng máy này để làm việc đó. Một lần chuyển bạn bắt đầu từ đây cho phiên của máy khác sẽ quay về máy này từ máy đó: khi đó việc mang theo sự tin cậy của thư mục hay các tệp git bỏ qua cần quyền **Nguy hiểm** mà mỗi máy cấp cho máy kia.',
          'Mọi thay đổi mà máy khác thực hiện ở đây đều được ghi vào nhật ký kiểm tra của máy này, kèm người đã thao tác qua máy đó; thao tác nguy hiểm còn hiện thông báo ở đây.',
          'Có quyền **Nguy hiểm**, máy kia có thể chạy bất cứ thứ gì ở đây: chỉ cấp cho máy tính bạn tin cậy như chính máy này.',
        ],
      },
    ],
  },
  {
    id: 'machines-sessions',
    title: 'Dùng phiên của máy khác',
    icon: 'sessions',
    group: 'machines',
    summary:
      'Xem và điều khiển phiên của mọi máy từ bất kỳ máy nào, và bắt đầu phiên trên máy khác.',
    intro:
      'Sau khi hai máy tính được ghép nối và tính năng Các máy được bật trên cả hai, mỗi máy liệt kê các phiên Claude Code của máy kia. Mở một phiên và dùng nó như thể nó chạy ở đây.',
    keywords: [
      'máy khác',
      'bộ lọc máy',
      'nhãn máy',
      'hộp thư',
      'ngoại tuyến',
      'nơi chạy',
      'phiên mới',
      'điện thoại',
      'điều khiển từ xa',
    ],
    sections: [
      {
        heading: 'Tìm các phiên',
        points: [
          '**Phiên** liệt kê phiên của mọi máy. Phiên của máy khác có nhãn ghi tên máy đó; phiên của máy này thì không.',
          'Bộ lọc **Máy** chọn một máy: **Mọi máy**, **máy Mac này** (hoặc **máy PC này**), hoặc một máy khác theo tên.',
          'Trên **Các máy**, thẻ của một máy cho biết máy đó có bao nhiêu phiên và bao nhiêu phiên đang chạy; **Mở các phiên của máy này** liệt kê chúng.',
          '**Hộp thư** liệt kê những gì đang chờ bạn ở mọi máy, mỗi yêu cầu kèm máy của nó. Hãy trả lời ở đây: câu trả lời đầu tiên được tính, như với điện thoại.',
        ],
        links: [{ label: 'Mở trang Phiên', path: '/' }],
      },
      {
        heading: 'Điều khiển các phiên',
        points: [
          'Mở một phiên: trang của nó ghi **Chạy trên … — bạn điều khiển nó từ đây.** Ô nhập tin nhắn, yêu cầu duyệt, dừng, mô hình, mức nỗ lực và chế độ hoạt động như với phiên của máy này.',
          'Khi máy kia không cấp quyền **Điều khiển**, trang ghi **Chạy trên … — bạn có thể xem nó từ đây.**, và việc bạn thử làm sẽ cho biết lý do, chẳng hạn **… chỉ cho máy Mac này xem các phiên của nó, không cho điều khiển.**',
          'Việc đổi tài khoản, chuyển hoặc sao chép phiên sang tài khoản khác hay kho lưu trữ, xóa phiên, mở tệp của phiên và mở phiên trong VS Code được thực hiện trên máy tính mà phiên đang chạy.',
        ],
      },
      {
        heading: 'Bắt đầu phiên trên máy khác',
        steps: [
          'Mở **Phiên mới**.',
          'Trong **Nơi chạy**, chọn máy kia.',
          'Chọn một dự án, hoặc nhập đường dẫn vào **Hoặc bất kỳ thư mục nào trên …** (ứng dụng máy tính).',
          'Nhấn **Bắt đầu trên …**.',
        ],
        points: [
          'Phiên chạy với tài khoản của máy đó cho thư mục này, và bạn điều khiển nó từ đây.',
          'Claude Code trên máy đó phải tin cậy thư mục này rồi: hãy mở nó bằng Claude Code trên máy đó một lần.',
        ],
        links: [{ label: 'Mở trang Phiên mới', path: '/new' }],
      },
      {
        heading: 'Khi một máy ngoại tuyến',
        points: [
          'Các phiên của máy đó vẫn được liệt kê, làm mờ, trong **Trên các máy tính đang ngoại tuyến**, cho đến khi máy trở lại.',
          'Trang của phiên cho biết máy của nó ngoại tuyến từ lúc nào. Những gì bạn thấy là từ lúc đó, và việc gửi sẽ chờ đến khi máy trở lại.',
          'Máy tính đang ngủ không trả lời được: **Giữ máy thức khi đang được dùng** giữ từng máy thức khi có máy khác đang dùng nó.',
        ],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại đã ghép nối với bất kỳ máy tính nào của bạn sẽ xem và điều khiển được mọi máy thông qua máy đó.',
          'Thao tác nguy hiểm trên máy khác cần quyền thao tác nguy hiểm của điện thoại trên máy mà nó ghép nối, và quyền **Nguy hiểm** mà máy kia cấp cho máy đó.',
          'Điện thoại ở trạng thái **Hạn chế** (tự làm mới vài giây một lần thay vì cập nhật trực tiếp) hiện chỉ thấy các phiên của máy mà nó kết nối.',
          'Việc thiết lập Các máy, ghép nối máy tính và thay đổi quyền của chúng được thực hiện trong ứng dụng máy tính.',
        ],
      },
    ],
  },
  {
    id: 'machines-move',
    title: 'Chuyển phiên sang máy tính khác',
    icon: 'move',
    group: 'machines',
    summary:
      'Tiếp tục một phiên trên máy tính khác, cùng mã nguồn của nó, rồi đưa về hoặc hoàn tác.',
    intro:
      'Một phiên có thể tiếp tục trên một máy tính khác của bạn: cùng cuộc trò chuyện, cùng mã nguồn, tiếp nối đúng chỗ đang dừng. Mỗi lúc phiên chỉ ở trên một máy; máy mà phiên rời đi giữ lại bản của nó, chỉ để đọc, và cho biết phiên giờ đang ở đâu.',
    keywords: [
      'chuyển phiên',
      'tiếp tục trên máy khác',
      'đưa về',
      'tiếp quản',
      'hoàn tác',
      'công thức',
      'mã nguồn',
      'đang ở trên',
    ],
    sections: [
      {
        heading: 'Chuyển phiên',
        steps: [
          'Mở menu **⋯** của phiên và chọn **Tiếp tục trên máy tính khác…**.',
          'Trong **Máy tính**, chọn nơi phiên sẽ đến. Mother Claude hỏi máy đó trước và hiện những gì nó tìm thấy: chưa có gì được ghi ở đâu cả.',
          'Xem lại **Những gì được chuyển**, **Nơi đến**, **Cách chuyển mã nguồn**, **Nơi đặt mã nguồn** và phần **Kiểm tra**.',
          'Nhấn **Tiếp tục trên …**.',
        ],
        tips: [
          'Với phiên của máy khác, menu có **Đưa về máy Mac này** (hoặc **Đưa về máy PC này**), hoặc **Tiếp tục trên máy Mac này…** cho phiên chưa từng ở đây. Một lần chuyển luôn bắt đầu trên máy mà phiên đang ở: từ đây, Mother Claude nhờ máy đó thực hiện.',
        ],
      },
      {
        heading: 'Các lựa chọn',
        terms: [
          {
            term: 'Máy tính',
            text: 'Các máy tính đã ghép nối đang trực tuyến và cho máy này chuyển phiên sang. Máy đang ngoại tuyến cho biết từ lúc nào.',
          },
          {
            term: 'Những gì được chuyển',
            text: 'Dung lượng cuộc trò chuyện, kèm các tác tử con, và mã nguồn: nhánh, số tệp đã thay đổi và số tệp mới.',
          },
          {
            term: 'Ngoài git',
            text: 'Các tệp git bỏ qua mà dự án liệt kê (**Tùy chọn › Từng dự án**), chẳng hạn `.env.local`. Chúng thường chứa bí mật, nên chỉ được mang theo khi có quyền thao tác nguy hiểm ở đây và quyền **Nguy hiểm** của máy nhận; bỏ đánh dấu **Mang theo chúng** để để chúng lại. Tệp cùng tên ở máy đó được giữ lại cho **Hoàn tác việc chuyển**.',
          },
          {
            term: 'Nơi đến',
            text: 'Thư mục của dự án trên máy kia, tìm qua thư mục dùng chung, cặp thư mục hoặc cùng đường dẫn. **Đổi…** để chọn thư mục khác; khi không tìm thấy, **Chọn thư mục của nó trên …** liệt kê các dự án của máy đó.',
          },
          {
            term: 'Cách chuyển mã nguồn',
            text: '**Trực tiếp giữa các máy** (mặc định: thẳng giữa hai máy, không đẩy lên đâu cả), **Qua remote git của bạn** (ref ẩn trên remote của bạn, bị xoá sau đó trừ khi được giữ lại) hoặc **Chỉ cuộc trò chuyện**. Qua remote, remote phải phản hồi trên cả hai máy mà không yêu cầu mật khẩu: việc này được kiểm tra trước khi chuyển.',
          },
          {
            term: 'Nơi đặt mã nguồn',
            text: '**Trong thư mục dự án** (mặc định, khi ở đó nó không có thay đổi riêng) hoặc **Trong thư mục riêng (worktree)**: một worktree của nó trong thư mục dữ liệu của Mother Claude ở máy đó; khi đó thư mục dự án không bao giờ bị đụng đến, và không có gì được gộp vào nó.',
          },
          {
            term: 'Sau khi chuyển',
            text: 'Khi dự án có công thức (đặt trong **Tùy chọn › Từng dự án**, hoặc tệp `.mother-claude/recipe.json` được commit trong dự án): nhờ Claude chạy các lệnh của nó trên máy kia, chẳng hạn `npm ci`. Chúng nằm trong tin nhắn đầu tiên Claude nhận ở đó và đi qua các yêu cầu duyệt của chính nó: Mother Claude không bao giờ tự chạy lệnh trên máy kia.',
          },
          {
            term: 'Khi nào',
            text: 'Khi một lượt đang chạy: **Sau lượt này** (mặc định: để lượt chạy xong trước) hoặc **Ngay bây giờ** (ngắt lượt hiện tại; những gì bạn đã xếp hàng đợi sẽ đi kèm).',
          },
          {
            term: 'Tiếp tục điều khiển nó từ đây',
            text: 'Bật theo mặc định: sau khi chuyển, trang này trở thành phiên ở nơi nó đang chạy.',
          },
          {
            term: 'Mang theo sự tin cậy của thư mục này sang …',
            text: 'Hiện khi Claude Code trên máy kia chưa từng mở thư mục đó. Thư mục được tin cậy sẽ chạy hook và máy chủ MCP của dự án mà không hỏi, nên đây là thao tác nguy hiểm: cần ứng dụng máy tính hoặc thiết bị được phép thực hiện thao tác nguy hiểm, và quyền **Nguy hiểm** mà máy nhận phiên cấp cho máy phiên rời đi.',
          },
        ],
      },
      {
        heading: 'Các mục kiểm tra',
        points: [
          '**Kiểm tra** liệt kê những gì ổn, những gì chặn việc chuyển kèm cách sửa, và những gì chỉ làm nó kém đi (Claude Code ở máy kia cũ hơn, kiểu xuống dòng, Git LFS, submodule). Tính năng Các máy phải được bật trên cả hai máy.',
          'Khi còn mục chặn, nút ghi **Hãy khắc phục các mục kiểm tra trước**. Khi thư mục dự án ở máy kia có thay đổi riêng, hoặc nhánh của nó đã đi tiếp ở đó, Mother Claude đề xuất **Dùng thư mục riêng (worktree)**.',
          'Phiên do ứng dụng khác chạy (VS Code, terminal) không thể bị Mother Claude dừng: một bản sao tiếp tục ở máy kia dưới dạng phiên mới, trong khi phiên này vẫn tiếp tục ở đây. Nút ghi **Tiếp tục dưới dạng bản sao trên …**; hoặc hãy kết thúc nó trong ứng dụng đó trước.',
        ],
      },
      {
        heading: 'Trong lúc chuyển',
        points: [
          'Từng bước hiện kèm thời gian: đã kiểm tra, đã dừng ở đây, đã đóng gói, đã gửi (bao nhiêu, và còn khoảng bao lâu), đã đặt lên máy kia, đã khởi động ở đó.',
          '**Hủy việc chuyển** dùng được cho đến khi phiên bắt đầu được đặt lên máy kia. **Đóng (vẫn tiếp tục chạy)** để việc chuyển tiếp tục; trang của phiên cho biết nó đang làm gì trong lúc đó.',
          'Kết nối bị rớt trong chốc lát khi đang gửi sẽ tiếp tục từ chỗ đã dừng.',
          'Tin nhắn đầu tiên phiên nhận ở máy kia cho Claude biết nó đã được chuyển: từ máy và thư mục nào sang đâu, rằng đường dẫn trong các tin nhắn trước là của máy cũ, và giờ nó dùng shell nào. Bản thân cuộc trò chuyện không bao giờ bị viết lại.',
        ],
      },
      {
        heading: 'Nếu có lỗi',
        terms: [
          {
            term: 'Trước khi được đặt lên máy kia',
            text: 'Không mất gì: phiên tiếp tục ở đây như trước (và khởi động lại nếu nó đang chạy), và máy kia bỏ những gì đã nhận. Mother Claude cho biết lý do; **Thử lại** đưa bạn về các lựa chọn.',
          },
          {
            term: 'Mã nguồn không thể đặt vào thư mục dự án',
            text: 'Mother Claude đề xuất một thư mục riêng (worktree) và hỏi lại.',
          },
          {
            term: 'Đã đặt lên nhưng chưa khởi động được ở đó',
            text: 'Chẳng hạn tài khoản của nó chưa đăng nhập ở đó. Giờ phiên đang ở máy đó: **Thử lại** khởi động nó ở đó mà không gửi lại gì; **Hoàn tác việc chuyển** đưa nó về.',
          },
        ],
      },
      {
        heading: 'Đang ở trên …',
        intro:
          'Máy mà phiên rời đi giữ lại bản của nó, làm mờ trong danh sách với nhãn **Đang ở trên …**. Trang của phiên cho biết phiên đang ở đâu và từ lúc nào, cùng các nút:',
        terms: [
          { term: 'Mở ở đó', text: 'Mở phiên ở nơi nó đang chạy.' },
          {
            term: 'Đưa nó về đây',
            text: 'Một lần chuyển về máy này, bắt đầu trên máy mà phiên đang ở, khi máy đó trực tuyến. Chỉ những gì đã diễn ra ở đó được thêm vào: bản trên máy này là phần đầu của cùng cuộc trò chuyện.',
          },
          {
            term: 'Tiếp quản…',
            text: 'Chỉ khi máy mà phiên đang ở ngoại tuyến: phiên tiếp tục ở đây từ bản của máy này, bản này chỉ có cuộc trò chuyện đến lúc nó được chuyển đi, với thư mục dự án của máy này như hiện tại. Nếu trong lúc đó máy kia vẫn tiếp tục phiên, cả hai phiên bản sẽ được giữ song song khi nó được chuyển sang đó lần tới.',
          },
        ],
        points: ['Không thể gửi tin nhắn tới bản ở đây; trang của phiên cho biết lý do.'],
      },
      {
        heading: 'Hoàn tác việc chuyển',
        steps: [
          'Tìm lần chuyển: ngay khi chuyển xong, trong **Các lần chuyển gần đây** trên trang **Các máy**, hoặc trong **Các lần chuyển giữa máy tính** ở thẻ **Thông tin** của phiên.',
          'Nhấn **Hoàn tác việc chuyển**.',
          'Nhấn **Chạm lần nữa để hoàn tác**.',
        ],
        points: [
          'Có thể hoàn tác trong bảy ngày. Việc hoàn tác đưa thư mục dự án và nhánh trên máy kia về như cũ, gỡ các tệp của cuộc trò chuyện ở đó ra, và phiên lại ở đây (khởi động lại ở đây nếu nó đang chạy).',
          'Không thể hoàn tác khi phiên đã tiếp tục ở máy kia: hãy đưa nó về, để không mất những gì nó đã làm ở đó. Bản sao thì không bao giờ được hoàn tác: hãy dừng hoặc xóa nó ở máy kia.',
        ],
      },
      {
        heading: 'Những gì không bao giờ được chuyển',
        points: [
          'Các tệp git bỏ qua, như tệp `.env`, `node_modules` và kết quả build, vẫn ở nguyên chỗ, trừ vài tệp mà dự án liệt kê để mang theo (**Ngoài git**). Hãy thiết lập dự án ở máy kia bằng công thức.',
          'Thông tin đăng nhập: token, khóa và `.credentials.json` ở lại trên từng máy. Phiên chạy với tài khoản cùng tên ở máy kia, đã đăng nhập ở đó.',
          'Worktree mà một lần chuyển tạo ra trên máy kia, trong thư mục dữ liệu của Mother Claude ở đó, vẫn ở lại khi phiên rời đi; công việc của nó đã đi cùng phiên. Hãy xóa nó ở máy đó.',
        ],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại đã ghép nối với bất kỳ máy tính nào của bạn cũng chuyển phiên theo cùng cách, trên một cột.',
          'Việc mang theo sự tin cậy của thư mục hay các tệp git bỏ qua cần quyền thao tác nguy hiểm của điện thoại, được đặt trong ứng dụng máy tính; không có quyền này thì tùy chọn sự tin cậy không hiện, và các tệp đó ở lại chỗ cũ.',
        ],
      },
    ],
  },

  // ──────────────────────────────────────────────────── Khắc phục sự cố
  {
    id: 'machines-sync',
    title: 'Giữ một dự án đồng bộ giữa hai máy tính',
    icon: 'refresh',
    group: 'machines',
    summary:
      'Mã nguồn của một dự án được giữ giống nhau trên hai máy tính, theo cả hai chiều: thay đổi trên máy này được chuyển sang máy kia.',
    intro:
      'Làm cùng một dự án trên hai máy tính? Đồng bộ mã nguồn giữ mã nguồn của nó giống nhau trên cả hai: một chỉnh sửa, một tệp mới, một commit hay một lần chuyển nhánh trên máy này sẽ xuất hiện trên máy kia sau vài giây. Tính năng này tắt cho đến khi bạn bật nó, cho từng dự án một, với một máy tính khác.',
    keywords: [
      'đồng bộ mã nguồn',
      'đồng bộ',
      'giữ đồng bộ',
      'hai máy tính',
      'cùng mã nguồn',
      'xung đột',
      'bản được giữ',
      'phản chiếu',
    ],
    sections: [
      {
        heading: 'Bật tính năng',
        where: 'desktop',
        steps: [
          'Mở **Các máy**, rồi **Chạy lại thiết lập** (hoặc **Thiết lập Các máy**), và đến bước **Tùy chọn**.',
          'Trong **Từng dự án**, nhấn **Đặt tùy chọn riêng cho một dự án…** rồi chọn dự án, hoặc nhấn **Thay đổi** ở một dự án đã có trong danh sách.',
          'Đánh dấu **Giữ dự án này đồng bộ với …** rồi nhấn **Lưu**.',
          'Trên máy tính kia, đề nghị hiện ở trang **Các máy** và dưới dạng thông báo: người ở máy đó nhấn **Giữ đồng bộ** (hoặc **Để sau**).',
        ],
        tips: [
          'Tính năng chỉ bật khi cả hai máy tính đều đồng ý. Lần đồng bộ đầu tiên chạy ngay sau đó: máy có thay đổi, hoặc có commit mới hơn, gửi chúng sang máy sạch và đang ở phía sau. Khi cả hai đều có thay đổi riêng, bạn chọn giữ bản nào (bên dưới).',
        ],
      },
      {
        heading: 'Những gì cần có',
        points: [
          'Một thư mục dùng chung ánh xạ dự án tới thư mục của nó trên máy tính kia (**Tài khoản và thư mục** trong thiết lập). Thư mục ở đó phải là cùng dự án: cùng `origin`, hoặc có chung một commit. Nếu dự án chưa có ở đó, hãy nhấn **Sao chép sang …** trước.',
          'Quyền **Di chuyển** theo cả hai chiều: đồng bộ mã nguồn ghi vào thư mục dự án của mỗi máy.',
          'Tính năng Các máy đang bật trên cả hai máy tính.',
        ],
      },
      {
        heading: 'Những gì được chuyển',
        points: [
          'Các commit của nhánh và các thay đổi đã stage, chưa stage và chưa theo dõi, đúng như hiện có: những gì một lần chuyển phiên mang theo. Không có gì bị gộp và không có gì được đẩy lên đâu cả: mã nguồn đi thẳng giữa hai máy tính.',
          'Các tệp git bỏ qua, như tệp `.env`, `node_modules` và kết quả build, ở nguyên trên mỗi máy, trừ những tệp dự án liệt kê trong **Các tệp git bỏ qua sẽ đi kèm**, và chỉ khi máy kia đã cấp cho máy này quyền **Nguy hiểm**.',
          'Vài giây yên lặng sau một thay đổi, máy đã thay đổi sẽ gửi nó đi; một lần kiểm tra mỗi phút bắt những gì bị sót. **Đồng bộ ngay** chạy một lần ngay lập tức.',
        ],
      },
      {
        heading: 'Thẻ Đồng bộ mã nguồn',
        intro:
          '**Các máy › Đồng bộ mã nguồn** liệt kê từng dự án được giữ đồng bộ, và tình trạng của nó:',
        terms: [
          { term: 'Đã đồng bộ', text: 'Lần đồng bộ gần nhất, và theo chiều nào.' },
          { term: 'Đang đồng bộ…', text: 'Các thay đổi đang được chuyển.' },
          {
            term: 'Đang chờ …',
            text: 'Máy tính kia đã được hỏi; chưa có gì được đồng bộ cho đến khi có người ở đó xác nhận.',
          },
          {
            term: 'Tạm dừng',
            text: 'Kèm lý do: máy tính kia đang ngoại tuyến, tính năng Các máy đang tắt trên một trong hai máy, thiếu một quyền **Di chuyển**, hoặc một phiên làm việc trong dự án đang được chuyển giữa hai máy. Nó tự tiếp tục khi điều đó kết thúc, từ dự án như lúc đó: không có gì bị xếp hàng chờ.',
          },
          {
            term: 'Thay đổi ở cả hai',
            text: 'Cả hai máy tính đều đã thay đổi nó: bạn chọn (bên dưới).',
          },
          {
            term: 'Tắt',
            text: 'Đã tắt trên máy tính kia, hoặc máy đó đã chọn để sau. **Bật lại** hỏi lại; **Gỡ bỏ** xóa nó khỏi máy này.',
          },
        ],
        points: ['**Tắt** dừng tính năng ngay trên máy này và báo cho máy kia.'],
      },
      {
        heading: 'Khi cả hai máy tính đều đã thay đổi',
        intro:
          'Đồng bộ mã nguồn không bao giờ ghi đè: khi cả hai máy tính đều đã thay đổi dự án kể từ lần đồng bộ trước, nó dừng trên cả hai, và thẻ cho biết mỗi bên đã thay đổi bao nhiêu tệp và có commit hay chuyển nhánh hay không.',
        steps: [
          'Nhấn **Giữ bản của máy Mac này** (hoặc **Giữ bản của máy PC này**) hoặc **Giữ bản của …** cho máy tính kia.',
          'Nhấn lần nữa để xác nhận.',
        ],
        points: [
          'Bên bạn không giữ trước tiên được lưu trong chính kho git của nó, dưới `refs/mother-claude/sync/<id>/kept-<time>`: cây làm việc cùng các tệp chưa theo dõi, những gì đã stage và các commit nó đã tạo. Sau đó thư mục dự án của nó trở về trạng thái đồng bộ gần nhất, và các thay đổi của bên được giữ sẽ được chuyển tới.',
          'Thẻ liệt kê từng trạng thái đã lưu. Để xem một trạng thái, trong thư mục dự án của máy đó: `git show --stat refs/mother-claude/sync/<id>/kept-<time>`.',
          'Cùng một thay đổi được thực hiện trên cả hai máy tính không phải là xung đột. Khi việc đặt một bên sang chỗ khác có thể làm mất thứ gì đó (một tệp git bỏ qua nằm cản đường, một submodule đã thay đổi, một lần merge đang dở), không có gì thay đổi và thẻ cho biết lý do.',
        ],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại thấy **Đồng bộ mã nguồn** và tình trạng của từng dự án, chỉ để đọc: bật hay tắt, **Đồng bộ ngay** và chọn một bên được thực hiện trong ứng dụng máy tính.',
        ],
      },
    ],
  },

  {
    id: 'machines-troubleshooting',
    title: 'Khắc phục sự cố Các máy',
    icon: 'question',
    group: 'troubleshooting',
    summary:
      'Khi hai máy tính không tìm thấy nhau, từ chối một thao tác, hoặc việc chuyển phiên dừng lại.',
    intro: 'Tìm thông báo hoặc hiện tượng bạn gặp, rồi xem cách khắc phục.',
    keywords: [
      'các máy',
      'ghép nối',
      'ở gần',
      'máy chủ chuyển tiếp',
      'ngoại tuyến',
      'quyền',
      'chuyển phiên',
      'tin cậy',
      'tài khoản',
      'lỗi',
    ],
    sections: [
      {
        heading: 'Thiết lập',
        terms: [
          {
            term: 'Bật tính năng Các máy bị làm mờ',
            text: 'Còn mục kiểm tra bắt buộc; dòng phía trên nút cho biết còn bao nhiêu. Hãy sửa chúng ở bước **Máy Mac này** (hoặc **Máy PC này**) và **Mọi máy**.',
          },
          {
            term: 'Cho phép qua tường lửa không thay đổi gì',
            text: 'Windows hỏi quản trị viên đồng ý trước. Nếu bị từ chối, tường lửa không bị thay đổi: hãy nhấn lại.',
          },
          {
            term: 'Chỉ các phiên của nó phải chờ: tính năng Các máy vẫn hoạt động.',
            text: 'Tài khoản đó chưa đăng nhập trên máy đó. Các phiên của nó phải chờ ở đó; mọi thứ khác vẫn hoạt động.',
          },
        ],
      },
      {
        heading: 'Ghép nối',
        terms: [
          {
            term: 'Máy kia không có trong Ở gần',
            text: 'Ở gần chỉ cho biết máy nào đang hiện mã: bạn luôn phải nhập mã, và mã dùng được cả khi khác mạng. Trên máy Mac, hãy kiểm tra quyền truy cập Mạng cục bộ của Mother Claude.',
          },
          {
            term: 'Không tìm thấy máy tính nào đang hiện mã này.',
            text: 'Hãy kiểm tra lại mã, và xem cả hai máy có Internet hoặc ở cùng một mạng không. Mỗi mã chỉ dùng được trong năm phút.',
          },
          {
            term: 'Mã đó không khớp.',
            text: 'Hãy kiểm tra lại trên máy tính kia, hoặc tạo mã mới ở đó.',
          },
          {
            term: 'Mã này đã bị hai máy tính dùng, nên không máy nào ghép nối được.',
            text: 'Hãy tạo mã mới.',
          },
          {
            term: 'Đó không phải dãy số trên …',
            text: 'Chưa có gì được ghép nối. Hãy bắt đầu lại, và nhập sáu chữ số mà máy kia hiển thị.',
          },
          {
            term: 'Lượt ghép nối không được xác nhận kịp.',
            text: 'Hãy xác nhận trên cả hai máy trong vòng mười phút kể từ khi tạo mã. Hãy bắt đầu lại.',
          },
        ],
      },
      {
        heading: 'Kết nối',
        terms: [
          {
            term: 'Qua máy chủ chuyển tiếp (chậm hơn), khi ở nhà',
            text: 'Trên máy Mac: quyền truy cập Mạng cục bộ. Trên Windows: loại mạng và tường lửa. **Tình trạng Các máy** cho biết mục nào.',
          },
          {
            term: '… dùng cài đặt máy chủ chuyển tiếp khác',
            text: 'Hai máy tính dùng máy chủ chuyển tiếp khác nhau có thể không bao giờ gặp được nhau. Nhấn **Dùng cả ở đây** trên một máy, hoặc **Dùng cài đặt này ở mọi nơi** trên máy kia.',
          },
          {
            term: 'Một máy đang ngoại tuyến',
            text: 'Máy đó đang ngủ, đã tắt, hoặc Mother Claude không chạy ở đó. Các phiên của nó vẫn được liệt kê, làm mờ, cho đến khi máy trở lại.',
          },
          {
            term: 'Cũ hơn bản trên máy tính này',
            text: 'Mother Claude trên máy đó cũ hơn: các phiên của nó vẫn hiện, nhưng không gì được chuyển đến hay đi khỏi máy đó cho đến khi bạn cập nhật.',
          },
          {
            term: 'Tính năng Các máy đang tắt trên …',
            text: 'Hãy bật nó trên trang **Các máy** của máy đó (hoặc của máy này, khi thông báo nói đến máy này). Khi tính năng này tắt, các máy kia chỉ ghép nối và chạy thiết lập được với máy đó.',
          },
        ],
      },
      {
        heading: 'Quyền',
        terms: [
          {
            term: '… chỉ cho máy tính này xem các phiên của nó, không cho điều khiển.',
            text: 'Quyền **Điều khiển** mà máy đó cấp cho máy này đang tắt: hãy đổi trên trang **Các máy** của máy đó.',
          },
          {
            term: '… không cho máy tính này thực hiện thao tác nguy hiểm trên đó.',
            text: 'Quyền **Nguy hiểm** mà máy đó cấp cho máy này đang tắt: hãy đổi trên máy đó. Điện thoại còn cần quyền thao tác nguy hiểm của chính nó trên máy mà nó ghép nối.',
          },
          {
            term: '… không cho máy tính này chuyển các phiên của nó.',
            text: 'Quyền **Di chuyển** mà máy đó cấp cho máy này đang tắt với loại mạng bạn đang dùng: hãy đổi trên máy đó.',
          },
          {
            term: '… đã từ chối máy tính này',
            text: 'Hãy kiểm tra trên máy đó xem hai máy còn được ghép nối không. Nếu máy đó đã thu hồi máy này, hãy ghép nối lại.',
          },
        ],
      },
      {
        heading: 'Chuyển phiên',
        terms: [
          {
            term: '… không có tài khoản nào tên …',
            text: 'Hãy thêm tài khoản cùng tên trên máy đó (**Tài khoản**) và đăng nhập. Phiên của tài khoản Mặc định sẽ tiếp tục với tài khoản Mặc định của máy đó.',
          },
          {
            term: '… chưa đăng nhập trên …',
            text: 'Hãy đăng nhập trên máy đó (trang **Tài khoản** ở đó), rồi thử lại.',
          },
          {
            term: '… chưa tin cậy …',
            text: 'Hãy mở thư mục đó một lần bằng Claude Code trên máy đó, hoặc mang theo sự tin cậy của thư mục (cần quyền thao tác nguy hiểm).',
          },
          {
            term: '… có thay đổi riêng',
            text: 'Hãy dùng thư mục riêng (worktree), hoặc commit hay stash các thay đổi ở đó trước.',
          },
          { term: '… đang chạy phiên này.', text: 'Hãy dừng nó trên máy đó trước.' },
          {
            term: 'Nó chạy không hỏi quyền',
            text: 'Phiên ở chế độ bỏ qua quyền cần quyền thao tác nguy hiểm ở đây, và quyền **Nguy hiểm** mà máy kia cấp cho máy này.',
          },
          {
            term: 'Remote … không phản hồi … mà không yêu cầu đăng nhập.',
            text: 'Hãy đăng nhập remote trên máy đó (Git Credential Manager, hoặc `gh auth login`) rồi thử lại, hoặc chuyển mã nguồn trực tiếp giữa hai máy.',
          },
          {
            term: 'Mang theo các tệp git bỏ qua cần …',
            text: 'Cần quyền thao tác nguy hiểm ở nơi phiên đang ở và quyền **Nguy hiểm** của máy nhận. Hoặc bỏ đánh dấu **Mang theo chúng**: các tệp ở lại chỗ cũ.',
          },
          {
            term: 'Không chuyển được …',
            text: 'Không mất gì: phiên vẫn ở nguyên chỗ, và thông báo cho biết lý do. **Thử lại** đưa bạn về các lựa chọn.',
          },
        ],
      },
    ],
  },
];

/** Vietnamese Help topics (docs/I18N.md "Help"), by topic id. */
export const VI_TOPICS_C: Partial<Record<string, HelpTopicSource>> = Object.fromEntries(
  TOPICS.map((t) => [t.id, t]),
);
