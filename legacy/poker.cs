using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Data;
using System.Drawing;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using System.Windows.Forms;

using System.Timers;
using System.Data.SqlClient;
using System.Net;
using System.Net.Sockets;
using System.IO;
using System.Net.NetworkInformation;
using System.Configuration;
using MySql.Data.MySqlClient;
using System.Security.Cryptography;
using System.Runtime.Remoting.Channels;
using System.Threading;

namespace pokerpractice
{
    public partial class poker : Form
    {
        UdpClient client;
        IPEndPoint IP_End;
        String deckstring, betsend;
        static Thread recieve_thread;
        string user = "", opponent = "", opponentIP = "", boardCards = "[", folderString = @"Hands\Hands " + DateTime.UtcNow.Year + DateTime.UtcNow.Month + DateTime.UtcNow.Day + DateTime.UtcNow.Hour + DateTime.UtcNow.Minute + DateTime.UtcNow.Second, handString = "";
        int userRank = 0, opponentRank = 0, minBet = 0, switcher = 1, cbet = 0, cbal = 0, pot = 0, round = -1, cpubet = 0, userbet = 0, game = 0,cardNum = 0, seat = 1;
        Boolean host = false, win = false, updated = false, clientConnected = false, muck = true;
        string connStr = ConfigurationManager.ConnectionStrings["connstr"].ConnectionString;
        int[] fulldeck = new int[52] { 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59 };
        int[] deck = new int[9];
        bool option = true, allin = false, dealer = false;
        public poker(string userName, string opp, string oppIP, Boolean hoster, int myRank, int oppRank)
        {
            InitializeComponent();
            /*user = userName;
            opponent = opp;
            opponentIP = oppIP;
            host = hoster;
            userRank = myRank;
            opponentRank = oppRank;*/
            //need to use connstr properly and scale images
        }
        byte[] receivedBytes;
        public void hostConnect()
        {
            client = new UdpClient(58310);
            IP_End = new IPEndPoint(IPAddress.Any, 58310);  

            byte[] response = Encoding.UTF8.GetBytes("Connected to server");   // Convert the reponse we want to send to the client to byte array
            client.Send(response, response.Length, IP_End);
            recieve_thread = new Thread(recv);
            recieve_thread.Start();
            // Receive the information from the client as byte array
            string clientMessage = Encoding.UTF8.GetString(receivedBytes);   // Convert the message to a string
            Array.Clear(receivedBytes,0,receivedBytes.Length);
            clientConnected = true;
            textBox2.AppendText(clientMessage);
            switcher = 0;
            backgroundWorker1.RunWorkerAsync(); //Start receiving data in background
            backgroundWorker2.WorkerSupportsCancellation = true;
        }

        void recv()
        {
            receivedBytes = client.Receive(ref IP_End);
            while (receivedBytes.Length == 0)
            {
                receivedBytes = client.Receive(ref IP_End);
            }
        }

        public async void clientConnect()
        {
            await Task.Delay(5000);
            string serverResponse = string.Empty;       // The variable which we will use to store the server response
            IP_End = new IPEndPoint(IPAddress.Parse(opponentIP), 58310);
            client = new UdpClient();
            byte[] data = Encoding.UTF8.GetBytes("Client Connected");      // Convert our message to a byte array
            client.Send(data, data.Length, IP_End);      // Send the date to the server

            serverResponse = Encoding.UTF8.GetString(client.Receive(ref IP_End));    // Retrieve the response from server as byte array and convert it to string
            if (serverResponse != string.Empty)
            {
                clientConnected = true;
                textBox2.AppendText(serverResponse);
            }
            try
            {
                if (clientConnected)
                {
                    switcher = 0;
                    backgroundWorker1.RunWorkerAsync();
                    backgroundWorker2.WorkerSupportsCancellation = true;
                    Deck_shuffle();
                    switcher = 2;
                    backgroundWorker2.RunWorkerAsync();
                }
            }
            catch (Exception x)
            {
                MessageBox.Show(x.Message.ToString());
            }
        }
        private void backgroundWorker1_DoWork(object sender, DoWorkEventArgs e) //RECIEVE DATA
        {
            while (clientConnected)
            {
                try
                {
                    recieve();
                }
                catch (Exception x)
                {
                    MessageBox.Show(x.Message.ToString());

                }
            }
            //if ranks not updated then ping google and update both ranks in our favour
            Ping ping = new Ping();
            PingReply pingReply = ping.Send("1.1.1.1");
            if (pingReply.Status == IPStatus.Success)
            {
                win = true;
                updateRanks();
                Button restart = new Button();
                restart.Click += new EventHandler(restart_Click);
                restart.Location = new Point(10, 100);
                restart.Text = "Back";
                restart.FlatStyle = FlatStyle.Flat;
                restart.BackColor = Color.White;
                panel1.Controls.Add(restart);
            }
        }
        public void recieve()
        {
            switch (switcher)
            {
                case 0: 
                    deckstring = "";
                    deckstring = Encoding.UTF8.GetString(client.Receive(ref IP_End));
                    if (deckstring.Contains(',') && !String.IsNullOrEmpty(deckstring))
                    {
                        switcher = 1;
                        this.textBox2.Invoke(new MethodInvoker(delegate () { textBox2.AppendText("CPU Deck: " + deckstring); }));
                        string[] array = new string[9];
                        array = deckstring.Split(',');
                        for (int i = 0; i < array.Length; i++)
                        {
                            Int32.TryParse(array[i], out deck[i]);
                        }
                        Invoke(new MethodInvoker(delegate ()
                        {
                            pictureBox1.Image = deckpics.Images[(deck[0] / 15) * 13 + deck[0] % 15 - 2];
                            pictureBox2.Image = deckpics.Images[(deck[1] / 15) * 13 + deck[1] % 15 - 2];
                        }));
                        deckstring = "";
                        if (game == 0)
                        {
                            Invoke(new MethodInvoker(delegate () {
                                seat = 2;
                                chip.Location = new Point(251, 359);
                                playgame(); 
                            }));
                        }
                    }
                    else if (!String.IsNullOrEmpty(deckstring))
                    {
                        betsend = deckstring;
                        switcher = 1;
                        choice();
                    }
                    break;
                case 1:
                    betsend = Encoding.UTF8.GetString(client.Receive(ref IP_End));
                    if (betsend.Contains(',') && !String.IsNullOrEmpty(betsend))
                    {
                        deckstring = betsend;
                        this.textBox2.Invoke(new MethodInvoker(delegate () { textBox2.AppendText("CPYOU Deck: " + deckstring); }));
                        string[] array = new string[9];
                        array = deckstring.Split(',');
                        for (int i = 0; i < array.Length; i++)
                        {
                            Int32.TryParse(array[i], out deck[i]);
                        }
                        Invoke(new MethodInvoker(delegate ()
                        {
                            pictureBox1.Image = deckpics.Images[(deck[0] / 15) * 13 + deck[0] % 15 - 2];
                            pictureBox2.Image = deckpics.Images[(deck[1] / 15) * 13 + deck[1] % 15 - 2];
                        }));
                        deckstring = "";
                        betsend = "";
                    }
                    else if (!String.IsNullOrEmpty(betsend))
                    {
                        choice();
                    }
                    break;
            }
        }
        public async void choice()
        {
            this.textBox2.Invoke(new MethodInvoker(delegate () { textBox2.AppendText("\nCPU bet: " + betsend + " userbet: " + userbet); }));
            cbet = System.Convert.ToInt32(betsend);
            betsend = "";

            cbal = System.Convert.ToInt32(cpubal.Text);
            if ((cbet != userbet && cbet > 0) || cbet == -2)
            {
                if (userbet > 0 || cbet == -2)
                {
                    if (cbet == -2)
                    {
                        cbet = minBet;
                    }
                    cpubet = cbet;
                    cbal = cbal - userbet - cpubet;
                    pot = pot + userbet + cpubet;
                    if (round == -1) { round++; }
                    Invoke(new MethodInvoker(delegate ()
                    {
                        cpuact.Text = "Raise " + cpubet;
                        historyBox.Text += System.Environment.NewLine + opponent + ": raises " + cpubet;
                        potlabel.Text = "" + pot;
                        cpubal.Text = "" + cbal;
                        callbutt.Visible = true;
                    }));
                }
                else
                {
                    cpubet = cbet;
                    pot = pot + cpubet;
                    Invoke(new MethodInvoker(delegate ()
                    {
                        historyBox.Text += System.Environment.NewLine + opponent + ": bets " + cpubet;
                        cpuact.Text = "Bet " + cpubet;
                        cbal = cbal - cpubet;
                        cpubal.Text = "" + cbal;
                        potlabel.Text = "" + pot;
                        callbutt.Visible = true;
                    }));
                }
                Invoke(new MethodInvoker(delegate ()
                {
                    userbuttons(true);
                }));
                minBet = cpubet;
                if (cbal == 0)
                {
                    allin = true;
                }
            }
            else if (cbet == userbet && cbet == 0)
            {
                Invoke(new MethodInvoker(delegate ()
                {
                    cpuact.Text = "Check";
                    historyBox.Text += System.Environment.NewLine + opponent + ": checks";
                    potlabel.Text = "" + pot;
                    userbuttons(true);
                    if (!dealer||pot==400) { check(); }
                }));
            }
            else if (cbet == -1)
            {
                Invoke(new MethodInvoker(delegate ()
                {
                    userbuttons(false);
                    userbal.Text = "" + (pot + System.Convert.ToInt32(userbal.Text));
                    historyBox.Text += "\n " + opponent + " Folds\n " + user + " wins: " + pot;
                    handhistory();
                }));
                await Task.Delay(3000);
                Invoke(new MethodInvoker(delegate ()
                {
                    playgame();
                }));
            }
            else if (cbet == -3)
            {
                muck = false;
            }
            else
            {
                Invoke(new MethodInvoker(delegate ()
                {
                    cpubet = userbet;
                    if (cpubet >= cbal)
                    {
                        cpubet = cbal;
                        allin = true;
                    }
                    pot = pot + cpubet;
                    cpuact.Text = "Call " + cpubet;
                    historyBox.Text += System.Environment.NewLine + opponent + ": calls " + cpubet;
                    cbal = cbal - cpubet;
                    cpubal.Text = "" + cbal;
                    potlabel.Text = "" + pot;
                    userbuttons(true);
                }));
                if (round == -1)
                {
                    round++;
                    userbet = 0;
                    cpubet = 0;
                }
                else { Invoke(new MethodInvoker(delegate () { check(); })); }
            }

            userbet = 0;
        }

        public void sendString(string str)
        {
            byte[] response = Encoding.UTF8.GetBytes(str);   // Convert the reponse we want to send to the client to byte array
            client.Send(response, response.Length, IP_End);
        }

        private void backgroundWorker2_DoWork(object sender, DoWorkEventArgs e) //SEND DATA
        {
            if (clientConnected)
            {
                switch (switcher)
                {
                    case 2:
                        sendString(deckstring);
                        this.textBox2.Invoke(new MethodInvoker(delegate () { textBox2.AppendText("Linus Deck: " + deckstring); }));
                        Invoke(new MethodInvoker(delegate ()
                        {
                            switcher = 1;
                            pictureBox1.Image = deckpics.Images[(deck[7] / 15) * 13 + deck[7] % 15 - 2];
                            pictureBox2.Image = deckpics.Images[(deck[8] / 15) * 13 + deck[8] % 15 - 2];
                        }));
                        if (game == 0)
                        {
                            Invoke(new MethodInvoker(delegate () { playgame(); }));
                        }
                        break;
                    case 0:
                    case 1:
                        betsend = System.Convert.ToString(userbet);
                        sendString(betsend);
                        this.textBox2.Invoke(new MethodInvoker(delegate () { textBox2.AppendText("\nUser bet: " + betsend); }));
                        if (userbet==-2) { userbet = minBet; }
                        break;
                }
            }
            else
            {
                MessageBox.Show("Send failed");
            }
            backgroundWorker2.CancelAsync();
        }

        private void Form1_Load(object sender, EventArgs e)
        {
            if (Directory.Exists(folderString)) { Directory.Delete(folderString, true); }
            Directory.CreateDirectory(folderString);
            //debug
            userRank = 1000;
            opponentRank = 1000;
            opponentIP = "192.168.56.1";
            //opponentIP = "90.240.236.236";
            using (StreamReader reader = new StreamReader(@"temp.txt"))
            {
                host = Convert.ToBoolean(reader.ReadLine());
            }
            if (host)
            {
                System.IO.File.WriteAllText(@"D:\Users\Linus\source\repos\thepokerladder\thepokerladder\bin\Debug\temp.txt", "false");
                user = "linus";
                opponent = "cpu";
            }
            else
            {
                user = "cpu";
                opponent = "linus";
                System.IO.File.WriteAllText(@"D:\Users\Linus\source\repos\thepokerladder\thepokerladder\bin\Debug\temp.txt", "true");
            }
            //endofdebug
            if (host)
            {
                hostConnect();
            }
            else
            {
                clientConnect();
            }
            /*using (var conn = new MySqlConnection(connStr))
            {
                using (var command = conn.CreateCommand())
                {
                    command.CommandText = @"
                        DELETE FROM dbo.match 
                        WHERE dbo.match.HostUser = @user OR dbo.match.ClientUser = @user;";
                    command.Parameters.AddWithValue("@user", user);
                    conn.Open();
                    command.ExecuteNonQuery();
                    conn.Close();
                }
            }*/
            label1.Text = opponent;
            label6.Text = user;
        }

        public async void playgame()
        {
            muck = true;
            allin = false;
            pot = 0;
            cardNum = 0;
            potlabel.Text = "" + pot;
            round = -1;
            boardCards = "";
            cpuact.Text = "...";
            useract.Text = "...";
            checkbutt.Visible = true;
            callbutt.Visible = false;
            showButton.Visible = false;
            game++;
            if (chip.Location.X == 550)
            {
                dealer = false;
                chip.Location = new Point(251, 359);
                historyBox.Text = "Pokerladder:  Hold'em No Limit (100/200) - " + DateTime.UtcNow.Year + "/" + DateTime.UtcNow.Month + "/" + DateTime.UtcNow.Day + " " + DateTime.UtcNow.TimeOfDay + " UTC" + System.Environment.NewLine + "Table 'TPL' 2-max (Play Money) Seat #" + Math.Abs(seat-3) + " is the button";
                userbuttons(false);
            }
            else
            {
                dealer = true;
                chip.Location = new Point(550, 372);
                historyBox.Text = "Pokerladder:  Hold'em No Limit (100/200) - " + DateTime.UtcNow.Year+ "/"+ DateTime.UtcNow.Month+ "/"+ DateTime.UtcNow.Day+ " "+ DateTime.UtcNow.TimeOfDay + " UTC" + System.Environment.NewLine + "Table 'TPL' 2-max (Play Money) Seat #" + seat + " is the button";
            }
            
            if (seat == 1)
            {
                historyBox.Text += System.Environment.NewLine + "Seat 1: " + user + " (" + userbal.Text + " in chips)"+ System.Environment.NewLine + "Seat 2: " + opponent + " (" + cpubal.Text + " in chips)";
            }
            else
            {
                historyBox.Text += System.Environment.NewLine + "Seat 1: " + opponent + " (" + cpubal.Text + " in chips)" + System.Environment.NewLine + "Seat 2: " + user + " (" + userbal.Text + " in chips)";
            }
            option = dealer;
            pictureBox3.Image = null;
            pictureBox4.Image = null;
            pictureBox5.Image = null; //reset all images
            pictureBox6.Image = null;
            pictureBox7.Image = null;
            pictureBox3.Visible = false;
            pictureBox4.Visible = false;
            pictureBox5.Visible = false;
            pictureBox6.Visible = false;
            pictureBox7.Visible = false;

            pictureBox9.Image = deckpics.Images[52];
            pictureBox10.Image = deckpics.Images[52];
            if (game != 1 && host)
            {
                switcher = 0;
            }
            await Task.Delay(1000);
            if (game != 1 && !host)
            {
                Invoke(new MethodInvoker(delegate ()
                {
                    backgroundWorker2.CancelAsync();
                    Deck_shuffle();
                    switcher = 2;
                    backgroundWorker2.RunWorkerAsync();
                }));
            }
            if (userbal.Text == "0")
            {
                historyBox.Text = opponent + " WINS THE GAME";
                updateRanks();
                Button restart = new Button();
                restart.Click += new EventHandler(restart_Click);
                restart.Location = new Point(10, 100);
                restart.Text = "Back";
                restart.FlatStyle = FlatStyle.Flat;
                restart.BackColor = Color.White;
                panel1.Controls.Add(restart);
            }
            else if (cpubal.Text == "0")
            {
                historyBox.Text = user + " WINS THE GAME";
                win = true;
                updateRanks();
                Button restart = new Button();
                restart.Click += new EventHandler(restart_Click);
                restart.Location = new Point(10, 100);
                restart.Text = "Back";
                restart.FlatStyle = FlatStyle.Flat;
                restart.BackColor = Color.White;
                panel1.Controls.Add(restart);
            }
            else
            {
                Preflop();
            }
        }
        public void updateRanks()
        {
            updated = true; //if someone disconnects then they forfeit
            if (win)
            {
                //rank go up
                userRank += (Int32)(30 * (1 - (1.0f * 1.0f / (1 + 1.0f * (float)(Math.Pow(10, 1.0f * (opponentRank - userRank) / 400))))));
                using (var conn = new MySqlConnection(connStr))
                {
                    using (var command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                                UPDATE dbo.ranks
                                SET dbo.ranks.UserRank = @rank
                                WHERE dbo.ranks.UserID = @userID";
                        command.Parameters.AddWithValue("@rank", userRank);
                        command.Parameters.AddWithValue("@userID", user);
                        conn.Open();
                        command.ExecuteNonQuery();
                        conn.Close();
                    }
                }
            }
            else
            {
                //rank go down
                userRank += (Int32)(30 * (0 - (1.0f * 1.0f / (1 + 1.0f * (float)(Math.Pow(10, 1.0f * (opponentRank - userRank) / 400))))));
                using (var conn = new MySqlConnection(connStr))
                {
                    using (var command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                                UPDATE dbo.ranks
                                SET dbo.ranks.UserRank = @rank
                                WHERE dbo.ranks.UserID = @userID";
                        command.Parameters.AddWithValue("@rank", userRank);
                        command.Parameters.AddWithValue("@userID", user);
                        conn.Open();
                        command.ExecuteNonQuery();
                        conn.Close();
                    }
                }
            }
        }
        public void restart_Click(object sender, System.EventArgs e)
        {
            Button button = sender as Button;
            //Back to search
            this.Hide();
            search frm = new search(user);
            frm.ShowDialog();
            this.Close();
        }
        private void poker_FormClosing(object sender, FormClosingEventArgs e)
        {
            try
            {
                if (!updated) { updateRanks(); }
                recieve_thread.Abort();
            }
            catch (Exception ex)
            {
                MessageBox.Show("Rank Update Failed: " + ex);
            }
        }
        public string Deck_shuffle()
        {
            Random rng = new Random();
            int temp;
            int n = 52;
            while (n > 1)
            {
                int k = rng.Next(n--);
                temp = fulldeck[n];
                fulldeck[n] = fulldeck[k];
                fulldeck[k] = temp;
            }
            deckstring = "" + fulldeck[0];
            deck[0] = fulldeck[0];
            for (int i = 1; i < 9; i++)
            {
                deckstring = deckstring + "," + fulldeck[i];
                deck[i] = fulldeck[i];
            }
            return deckstring;
        }

        public void Preflop()
        {
            int userBalance = System.Convert.ToInt32(userbal.Text),cpuBalance = System.Convert.ToInt32(cpubal.Text);
            if (host)
            {
                cardNum = 7;
            }
            if (!dealer) //dealer is small non dealer is big
            {
                userbet = Math.Min(Math.Min(200,userBalance),cpuBalance);
                cpubet = Math.Min(Math.Min(100, userBalance), cpuBalance);
                minBet = userbet;
                historyBox.Text += System.Environment.NewLine + opponent + ": posts small blind " + cpubet + System.Environment.NewLine + user + ": posts big blind " + userbet + System.Environment.NewLine + "*** HOLE CARDS ***";
                cpuact.Text = "Small Blind " + cpubet;
                useract.Text = "Big Blind " + userbet;
                pot = userbet + cpubet;
                potlabel.Text = "" + pot;
                cpubal.Text = "" + (cpuBalance - cpubet);
                userbal.Text = "" + (userBalance - userbet);
                userbet = 100;
                cpubet = 0;
            }
            else
            {
                userbet = Math.Min(Math.Min(100, userBalance), cpuBalance);
                cpubet = Math.Min(Math.Min(200, userBalance), cpuBalance);
                minBet = cpubet;
                checkbutt.Visible = false;
                callbutt.Visible = true;
                historyBox.Text += System.Environment.NewLine + user + ": posts small blind " + userbet + System.Environment.NewLine + opponent + ": posts big blind " + cpubet + System.Environment.NewLine + "*** HOLE CARDS ***";
                cpuact.Text = "Big Blind " + cpubet;
                useract.Text = "Small Blind " + userbet;
                pot = userbet + cpubet;
                potlabel.Text = "" + pot;
                cpubal.Text = "" + (System.Convert.ToInt32(cpubal.Text) - cpubet);
                userbal.Text = "" + (System.Convert.ToInt32(userbal.Text) - userbet);
                cpubet = 100;
                userbet = 0;
            }
            historyBox.Text += System.Environment.NewLine + "Dealt to "+user+" ["+outputcards(Math.Abs(cardNum-7),deck)+" "+ outputcards(Math.Abs(cardNum - 7) + 1, deck) + "]";
            if (option) { userbuttons(true); }
            else { userbuttons(false); };
        }
        public void Flop()
        {
            round = 1;
            boardCards += outputcards(2, deck) + " " + outputcards(3, deck) + " " + outputcards(4, deck);
            historyBox.Text += System.Environment.NewLine + "*** FLOP *** [" + boardCards +"]";
            pictureBox3.Image = deckpics.Images[(deck[2] / 15) * 13 + deck[2] % 15 - 2];
            pictureBox4.Image = deckpics.Images[(deck[3] / 15) * 13 + deck[3] % 15 - 2];
            pictureBox5.Image = deckpics.Images[(deck[4] / 15) * 13 + deck[4] % 15 - 2];
            pictureBox3.Visible = true;
            pictureBox4.Visible = true;
            pictureBox5.Visible = true;
            checkbutt.Visible = true;
            checkturn();
        }

        public void Turn()
        {
            round++;
            historyBox.Text += System.Environment.NewLine + "*** TURN *** [" + boardCards+ "] [" + outputcards(5, deck) + "]";
            boardCards += " "+outputcards(5, deck);
            pictureBox6.Image = deckpics.Images[(deck[5] / 15) * 13 + deck[5] % 15 - 2];
            pictureBox6.Visible = true;
            checkturn();
        }
        public void River()
        {
            round++;
            historyBox.Text += System.Environment.NewLine + "*** RIVER *** [" + boardCards + "] [" + outputcards(6, deck) + "]";
            boardCards += " "+outputcards(6, deck);
            pictureBox7.Image = deckpics.Images[(deck[6] / 15) * 13 + deck[6] % 15 - 2];
            pictureBox7.Visible = true;
            checkturn();
        }
        private void showButton_Click(object sender, EventArgs e)
        {
            showButton.Visible = false;
            userbet = -3;
            historyBox.Text += System.Environment.NewLine + user + ": shows [" + outputcards(Math.Abs(cardNum - 7), deck) + " " + outputcards(Math.Abs(cardNum - 7) + 1, deck) + "]";
            backgroundWorker2.RunWorkerAsync();
        }
        public void showCards()
        {
            pictureBox9.Image = deckpics.Images[(deck[cardNum] / 15) * 13 + deck[cardNum] % 15 - 2];
            pictureBox10.Image = deckpics.Images[(deck[cardNum + 1] / 15) * 13 + deck[cardNum + 1] % 15 - 2];
        }
        public async void Reveal()
        {
            int winType = 1;
            bool dealerLoss = true;
            string userString = "", userSummary = "", cpuSummary = "", potString = "";
            int[] userhand = new int[6];
            int[] cpuhand = new int[6];
            if (host)
            {
                switcher = 0;
            }
            historyBox.Text += System.Environment.NewLine + "*** SHOWDOWN ***";
            userhand = checkhand(Math.Abs(cardNum-7));
            userString = handString;
            cpuhand = checkhand(cardNum);
            if (userhand[5] > cpuhand[5])
            {
                winType = 2;
                userbal.Text = "" + (pot + System.Convert.ToInt32(userbal.Text));
            }
            else if (userhand[5] < cpuhand[5])
            {
                winType = 0;
                cpubal.Text = "" + (pot + System.Convert.ToInt32(cpubal.Text));
            }
            else
            {
                for (int i = 0; i < 5; i++)
                {
                    if (userhand[i] > cpuhand[i])
                    {
                        winType = 2;
                        userbal.Text = "" + (pot + System.Convert.ToInt32(userbal.Text));
                        break;
                    }
                    else if (userhand[i] < cpuhand[i])
                    {
                        winType = 0;
                        cpubal.Text = "" + (pot + System.Convert.ToInt32(cpubal.Text));
                        break;
                    }
                }
            }
            if (dealer)
            {
                //show opps cards
                showCards();
                historyBox.Text += System.Environment.NewLine + opponent + ": shows [" + outputcards(Math.Abs(cardNum), deck) + " " + outputcards(Math.Abs(cardNum) + 1, deck) + "] (" + handString + ")";
                
                if (winType == 0)
                    showButton.Visible = true;
                else
                {
                    dealerLoss = false;
                    historyBox.Text += System.Environment.NewLine + user + ": shows [" + outputcards(Math.Abs(cardNum - 7), deck) + " " + outputcards(Math.Abs(cardNum - 7) + 1, deck) + "] (" + userString + ")";
                }
            }
            else if (winType != 2)
            {
                historyBox.Text += System.Environment.NewLine + user + ": shows [" + outputcards(Math.Abs(cardNum - 7), deck) + " " + outputcards(Math.Abs(cardNum - 7) + 1, deck) + "] (" + userString + ")";
                historyBox.Text += System.Environment.NewLine + opponent + ": shows [" + outputcards(Math.Abs(cardNum), deck) + " " + outputcards(Math.Abs(cardNum) + 1, deck) + "] (" + handString + ")";
                showCards();
            }
            else
            {
                historyBox.Text += System.Environment.NewLine + user + ": shows [" + outputcards(Math.Abs(cardNum - 7), deck) + " " + outputcards(Math.Abs(cardNum - 7) + 1, deck) + "] (" + userString + ")";
            }
            switch (winType)
            {
                case 0:
                    cpuSummary = "[" + outputcards(Math.Abs(cardNum), deck) + " " + outputcards(Math.Abs(cardNum) + 1, deck) + "] and won (" + pot + ") with " + handString;
                    userSummary = "[" + outputcards(Math.Abs(cardNum-7), deck) + " " + outputcards(Math.Abs(cardNum-7) + 1, deck) + "] and lost with " + handString;
                    potString = opponent + ": collected " + pot + " from pot";
                    break;
                case 1:
                    cpuSummary = "[" + outputcards(Math.Abs(cardNum), deck) + " " + outputcards(Math.Abs(cardNum) + 1, deck) + "] and won (" + pot/2 + ") with " + handString;
                    userSummary = "["+outputcards(Math.Abs(cardNum - 7), deck) + " " + outputcards(Math.Abs(cardNum - 7) + 1, deck) + "] and won (" + pot / 2 + ") with " + handString;
                    potString = user + ": collected " + (pot / 2) + " from pot" + System.Environment.NewLine + opponent + ": collected " + (pot / 2) + " from pot";
                    userbal.Text = "" + (pot / 2 + System.Convert.ToInt32(userbal.Text));
                    cpubal.Text = "" + (pot / 2 + System.Convert.ToInt32(cpubal.Text));
                    break;
                case 2:
                    cpuSummary = "[" + outputcards(Math.Abs(cardNum), deck) + " " + outputcards(Math.Abs(cardNum) + 1, deck) + "] and lost with " + handString;
                    userSummary = "[" + outputcards(Math.Abs(cardNum - 7), deck) + " " + outputcards(Math.Abs(cardNum - 7) + 1, deck) + "] and won (" + pot / 2 + ") with " + handString;
                    potString = user + ": collected " + pot + " from pot";
                    break;
            }
            await Task.Delay(3000);
            showButton.Visible = false;
            if (!muck && !dealerLoss)
            {
                historyBox.Text += System.Environment.NewLine + opponent + ": shows [" + outputcards(Math.Abs(cardNum), deck) + " " + outputcards(Math.Abs(cardNum) + 1, deck) + "] (" + handString + ")";
                showCards();
            }
            else
            {
                cpuSummary = "mucked";
                historyBox.Text += System.Environment.NewLine + opponent +": mucks hand";
            }
            if (userbet == -3)
            {
                
                userSummary = "mucked [" + outputcards(Math.Abs(cardNum - 7), deck) + " " + outputcards(Math.Abs(cardNum - 7) + 1, deck) + "]";
            }
            historyBox.Text += System.Environment.NewLine + potString;
            summaryOutput(cpuSummary,userSummary);
            handhistory();
            await Task.Delay(1000);
            Invoke(new MethodInvoker(delegate () { playgame(); }));
        }
        public void summaryOutput(string cpuSumm, string userSumm)
        {
            historyBox.Text += System.Environment.NewLine + "*** SUMMARY ***"+System.Environment.NewLine+"Total pot "+pot+System.Environment.NewLine+"Board ["+boardCards+"]";
            if (seat == 1)
            {
                if (dealer)
                    historyBox.Text += System.Environment.NewLine + "Seat 1: "+user+" (button) (big blind) "+userSumm+System.Environment.NewLine+ "Seat 2: " + opponent + " (small blind) " + cpuSumm;
                else
                    historyBox.Text += System.Environment.NewLine + "Seat 1: " + user + " (small blind) " + userSumm + System.Environment.NewLine + "Seat 2: " + opponent + " (button) (big blind) " + cpuSumm;
            }
            else
            {
                if (dealer)
                    historyBox.Text += System.Environment.NewLine + "Seat 1: " + opponent + " (button) (big blind) " + cpuSumm + System.Environment.NewLine + "Seat 2: " + user + " (small blind) " + userSumm;
                else
                    historyBox.Text += System.Environment.NewLine + "Seat 1: " + opponent + " (small blind) " + cpuSumm + System.Environment.NewLine + "Seat 2: " + user + " (button) (big blind) " + userSumm;
            }
        }
        public void checkturn()
        {
            cpuact.Text = "";
            useract.Text = "";
            if (!allin)
            {
                if (dealer)
                {
                    option = true;
                    userbuttons(false);
                }
                else
                {
                    option = false;
                    userbuttons(true);
                }
            }
            else
            {
                userbuttons(false);
                showCards();
                check();
            }
        }

        public void handhistory()
        {
            using (StreamWriter writer = File.AppendText(folderString+"\\Hand " + (game) + ".txt"))
            {
                writer.Write(historyBox.Text);
            }
            Button newButton = new Button();
            newButton.Tag = "" + (game);
            newButton.Click += new EventHandler((s, e) => newButton_Click(s, e, System.Convert.ToInt32(newButton.Tag)));
            newButton.Width = 25;
            newButton.Height = 25;
            Table.Controls.Add(newButton, 3, game - 1);
            Label potlabel = new Label();
            potlabel.ForeColor = Color.White;
            potlabel.Text = "" + pot;
            Table.Controls.Add(potlabel, 2, game - 1);
            Label boardlabel = new Label();
            boardlabel.AutoSize = true;
            boardlabel.Size = new Size(111, 13);
            boardlabel.ForeColor = Color.White;
            if (round != 0)
            { //board index: 2,3,4,5,6 
                for (int i = 2; i < round + 4; i++) { boardlabel.Text +=  " " + outputcards(i, deck); }
            }
            textBox2.AppendText(boardlabel.Text);
            Table.Controls.Add(boardlabel, 1, game - 1);
            Label handlabel = new Label();
            handlabel.ForeColor = Color.White;
            handlabel.Text = "  " + outputcards(Math.Abs(cardNum-7), deck) + " " + outputcards(Math.Abs(cardNum - 7)+1, deck);
            Table.Controls.Add(handlabel, 0, game - 1);
        }

        private void newButton_Click(object sender, EventArgs e, int num)
        {
            System.Diagnostics.Process.Start(folderString+"\\Hand " + num + ".txt");
        }

        public int[] checkhand(int cnum)
        {
            int[] hand = new int[6];
            int[] cards = new int[14];
            int[] suits = new int[4];
            int[] sort = new int[7] { deck[cnum], deck[cnum + 1], deck[2], deck[3], deck[4], deck[5], deck[6] };
            int pairs = 0, top = 0, suit = 0, st = 0, value = 0, toppair = 0, lowpair = 0, t = 0, first = 0, consec = 0, previous = 0, rank = 0, trip = 0, suitcount = 0;
            for (int c = 6; c > 0; c--)
            {
                first = 0;
                for (int d = 1; d <= c; d++)
                {
                    if (sort[d] % 15 < sort[first] % 15) { first = d; }
                }
                t = sort[first];
                sort[first] = sort[c];
                sort[c] = t;
            }
            /*if (cnum == 0) {
                for (int h = 0; h < 7; h++)
                {
                    deck.WriteLine("\n" + outputvalue(sort[h] % 15));
                }
            }FOR DEBUGGING*/
            for (int i = 0; i < 7; i++)
            {
                st = sort[i] / 15;
                value = sort[i] % 15;
                cards[value - 2]++;
                suits[st]++;
                if (suits[st] == 5) //flush
                {
                    if (rank < 5)
                    {
                        rank = 5;
                        suit = st;
                        top = 4 + value;
                    }
                }
                if (value == previous - 1 || previous == 0 || (value == 2 && sort[0] % 15 == 14 && consec == 4)) //straight
                {
                    if (previous / 15 == sort[i] / 15)
                    {
                        suitcount++;
                    }
                    else
                    {
                        suitcount = 0;
                    }
                    previous = value;
                    consec++;
                    if (consec == 5)
                    {
                        if (suitcount > 4)
                        {
                            rank = 8;
                            if (top != 5) { top = 4 + value; }
                            if (top == 14) { handString = "royal flush, " + outputString(14) + " to " + outputString(10); }
                            else if (top == 5) { handString = "straight flush, " + outputString(top) + " to " + outputString(14); }
                            else { handString = "straight flush, " + outputString(top-4) + " to " + outputString(top); }
                            break;
                        }
                        if (rank < 4)
                        {
                            rank = 4;
                            if (top != 5) { top = 4 + value; }
                        }
                    }
                }
                else if (previous != value)
                {
                    previous = 0;
                    consec = 0;
                }
                if (cards[value - 2] > 3)
                {
                    //quads for sure
                    rank = 7;
                    handString = "four of a kind, " + outputString(value) + "s";
                    hand[0] = value;
                    break;
                }
                else if (cards[value - 2] > 2)
                {
                    //trips, check for higher trip and full house
                    if (rank < 3) { rank = 3; }
                    if (toppair == value && trip == 0)
                    {
                        toppair = lowpair;
                    }
                    trip = value;
                    lowpair = 0;
                    pairs--;
                    if (toppair > 0)
                    {
                        rank = 6;
                        handString = "full house, " + outputString(trip) + "s full of " + outputString(toppair) + "s";
                        break;
                    }
                }
                else if (cards[value - 2] > 1)
                {
                    //pair or two pair (check for higher two pair)
                    pairs++;
                    if (pairs == 3) { pairs = 2; }
                    if (rank < pairs) { rank = pairs; }
                    if (toppair == 0) { toppair = value; }
                    else if (lowpair == 0) { lowpair = value; }
                    if (trip > 0)
                    {
                        rank = 6;
                        break;
                    }
                }
            }
            //look through array and where item isnt pair populate with high cards
            int j = 0, k = 0;
            switch (rank)
            {
                case 0:
                    handString = "high card, " + outputString(sort[0] % 15);
                    break;
                case 1:
                    hand[0] = toppair;
                    hand[1] = toppair;
                    j = 2;
                    handString = "pair, " + outputString(toppair) + "s";
                    break;
                case 2:
                    hand[0] = toppair;
                    hand[1] = toppair;
                    hand[2] = lowpair;
                    hand[3] = lowpair;
                    j = 4;
                    handString = "two pair, " + outputString(toppair) + "s and " + outputString(lowpair) + "s";
                    break;
                case 3:
                case 6:
                    handString = "three of a kind, " + outputString(trip) + "s";
                    hand[0] = trip;
                    hand[1] = trip;
                    hand[2] = trip;
                    j = 3;
                    if (rank == 6)
                    {
                        hand[3] = toppair;
                        hand[4] = toppair;
                        j = 5;
                    }
                    break;
                case 4:
                    if (top == 5) { handString = "straight, " + outputString(top) + " to " + outputString(14); }
                    else { handString = "straight, " + outputString(top-4) + " to " + outputString(top); }
                    j = 5;
                    hand[0] = top;
                    break;
                case 5:
                    handString = "flush, " + top + " high";
                    j = 5;
                    hand[0] = top;
                    break;
            }
            for (int i = j; i < 5; i++)
            {
                while (hand[i] == 0)
                {
                    if ((sort[k] % 15) != trip && (sort[k] % 15) != toppair && (sort[k] % 15) != lowpair)
                    {
                        hand[i] = sort[k] % 15;
                    }
                    k++;
                }
            }
            handString.Replace("Sixs","Sixes");
            hand[5] = rank;
            return hand;
        }

        private void callbutt_Click(object sender, EventArgs e)
        {
            userbuttons(false);
            if (userbet >= System.Convert.ToInt32(userbal.Text))
            {
                userbet = System.Convert.ToInt32(userbal.Text);
                allin = true;
                showCards();
            }
            userbet = cpubet;
            pot += userbet;
            potlabel.Text = "" + pot;
            useract.Text = "Call " + userbet;
            historyBox.Text += System.Environment.NewLine + user + ": calls " + userbet;
            userbal.Text = "" + (System.Convert.ToInt32(userbal.Text) - userbet);
            callbutt.Visible = false;
            checkbutt.Visible = true;
            if (dealer && round == -1 && !allin)
            {
                backgroundWorker2.RunWorkerAsync();
                round++;
                userbet = 0;
                cpubet = 0;
            }
            else
            {
                check();
                backgroundWorker2.CancelAsync();
                backgroundWorker2.RunWorkerAsync();
            }
        }

        public string outputcards(int num, int[] arr)
        {
            string card = "";
            card = outputvalue((arr[num] % 15)) + outputsuit((arr[num] / 15));
            return card;
        }

        public char outputsuit(int rank)
        {
            char suit;
            switch (rank)
            {
                case 0:
                    suit = 'c';
                    break;
                case 1:
                    suit = 'd';
                    break;
                case 2:
                    suit = 'h';
                    break;
                default:
                    suit = 's';
                    break;
            }
            return suit;
        }
        public string outputString(int rank)
        {
            switch (rank)
            {
                case 2:
                    return "Deuce";
                case 3:
                    return "Three";
                case 4:
                    return "Four";
                case 5:
                    return "Five";
                case 6:
                    return "Six";
                case 7:
                    return "Seven";
                case 8:
                    return "Eight";
                case 9:
                    return "Nine";
                case 10:
                    return "Ten";
                case 11:
                    return "Jack";
                case 12:
                    return "Queen";
                case 13:
                    return "King";
                default:
                    return "Ace";
            }
        }
            public string outputvalue(int rank)
        {
            string value;
            switch (rank)
            {
                case int n when n < 10:
                    value = System.Convert.ToString(rank);
                    break;
                case 10:
                    value = "T";
                    break;
                case 11:
                    value = "J";
                    break;
                case 12:
                    value = "Q";
                    break;
                case 13:
                    value = "K";
                    break;
                default:
                    value = "A";
                    break;
            }
            return value;
        }

        private void trackBar1_Scroll(object sender, EventArgs e)
        {
            int trackValue;
            bool blockRecursion = false;
            if (blockRecursion) return;
            trackValue = trackBar1.Value;
            if (trackValue % 100 != 0)
            {
                trackValue = (trackValue / 100) * 100;
                blockRecursion = true;
                trackBar1.Value = trackValue;
                blockRecursion = false;
            }
            betbox.Text = System.Convert.ToString(trackValue);
        }

        private async void foldbutt_Click(object sender, EventArgs e)
        {
            Invoke(new MethodInvoker(delegate ()
            {
                cpubal.Text = "" + (pot + System.Convert.ToInt32(cpubal.Text));
                historyBox.Text += "\n " + user + " Folds\n " + opponent + " wins: " + pot;
            }));
            userbet = -1;
            backgroundWorker2.RunWorkerAsync();
            Invoke(new MethodInvoker(delegate ()
            {
                handhistory();
                callbutt.Visible = false;
                userbuttons(false);
            }));
            await Task.Delay(3000);
            Invoke(new MethodInvoker(delegate () {
                playgame(); 
            }));
        }

        public void check()
        {
            userbet = 0;
            cpubet = 0;
            minBet = 200;
            switch (round)
            {
                case 0:
                    Flop();
                    break;
                case 1:
                    Turn();
                    break;
                case 2:
                    River();
                    break;
                case 3:
                    userbuttons(false);
                    Reveal();
                    break;
            }
        }

        public void userbuttons(bool type)
        {
            betbox.Enabled = type;
            trackBar1.Minimum = Math.Min(Math.Min(minBet, System.Convert.ToInt32(userbal.Text)), System.Convert.ToInt32(cpubal.Text));
            trackBar1.Maximum = System.Convert.ToInt32(userbal.Text) - cpubet;
            trackBar1.Enabled = type;
            checkbutt.Enabled = type;
            foldbutt.Enabled = type;
            if (!type)
            {
                betbox.Text = "";
            }
        }

        private void checkbutt_Click_1(object sender, EventArgs e)
        {
            userbuttons(false);
            useract.Text = "Check";
            historyBox.Text += System.Environment.NewLine + user + ": checks";
            backgroundWorker2.RunWorkerAsync();
            if(dealer || pot == 400)
                check();  
        }

        private void betbutt_Click(object sender, EventArgs e)
        {
            int dif = cpubet - userbet;
            userbet = System.Convert.ToInt32(betbox.Text);
            int ubal = System.Convert.ToInt32(userbal.Text);
            callbutt.Visible = false;
            userbuttons(false);
            minBet = userbet;
            if (cpubet > 0)
            {
                useract.Text = "Raise " + userbet;
                historyBox.Text += System.Environment.NewLine + user + ": raises " + userbet;
                ubal = ubal - userbet - dif;
                pot = pot + userbet + dif;
                potlabel.Text = "" + pot;
                userbal.Text = "" + ubal;
                if (userbet==cpubet) { userbet = -2; }
                if (round == -1) { round++; }
            }
            else
            {
                pot = pot + userbet;
                historyBox.Text += System.Environment.NewLine + user + ": bets " + userbet;
                useract.Text = "Bet " + userbet;
                userbal.Text = "" + (ubal - userbet);
            }
            if (ubal == userbet)
            {
                allin = true;
            }
            potlabel.Text = "" + pot;
            cpubet = 0;
            switcher = 1;
            backgroundWorker2.RunWorkerAsync();
        }

        private void betbox_TextChanged(object sender, EventArgs e)
        {
            String input = betbox.Text;
            int newbet = 0;
            try
            {
                newbet = System.Convert.ToInt32(input);
            }
            catch (FormatException)
            {
                betbox.Text = "";
            }
            int lowestBal = Math.Min(System.Convert.ToInt32(userbal.Text) - cpubet, System.Convert.ToInt32(cpubal.Text));
            int minimum = Math.Min(minBet, lowestBal);
            if (betbox.TextLength != 0 && betbox.TextLength < 9)
            {
                newbet = System.Convert.ToInt32(input);
                if (minimum != minBet)
                {
                    betbox.Text = "" + minimum;
                    trackBar1.Enabled = false;
                    betbutt.Enabled = true;
                }
                else if ((newbet >= minBet && newbet <= lowestBal)) { 
                    betbutt.Enabled = true;
                }  
                else
                {
                    betbutt.Enabled = false;
                }
            }
            else
            {
                betbox.Text = "";
                betbutt.Enabled = false;
            }
        }
    }
}